mod actions;
mod app;
mod clipboard;
mod theme;
mod ui;

use std::io::{self, stdout, Write};
use std::panic;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind, PopKeyboardEnhancementFlags,
    },
    execute,
    style::ResetColor,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_subscriber::EnvFilter;

use app::{DetailTab, ExternalRequest, Overlay, SettingsCursor, TuiApp, ViewMode};

#[tokio::main]
async fn main() -> io::Result<()> {
    init_tui_tracing();

    rl_core::ensure_plugins_dir();
    rl_core::write_example_manifest_if_missing();

    // SIGINT/SIGTERM skip Rust Drop unless we restore explicitly first.
    let _ = ctrlc::set_handler(|| {
        restore_terminal();
        std::process::exit(130);
    });

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).inspect_err(|_| restore_terminal())?;

    // Always restore termios / mouse / alt-screen — including on panic and early returns.
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
    }));
    let _terminal_guard = TerminalRestoreGuard;

    let mut app = TuiApp::new();
    let result = run(&mut terminal, &mut app).await;

    for (_, session) in app.port_forwards.drain() {
        session.stop();
    }

    // Restore before Terminal Drop / runtime teardown.
    drop(_terminal_guard);
    let _ = terminal.show_cursor();
    result
}

/// Ensures mouse tracking / raw mode / alt-screen are cleared even if cleanup is skipped.
struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    // CRITICAL ORDER: disable mouse *before* leaving raw mode. If raw mode is cleared
    // first, in-flight SGR mouse reports (`\x1b[<…M`) land in the shell as phantom typing.
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange,
        PopKeyboardEnhancementFlags,
    );
    // Belt-and-suspenders: some terminals keep a private mode if only one disable ran.
    write_tty_bytes(
        b"\x1b[?1000l\x1b[?1001l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l\x1b[?1004l\x1b[?2004l",
    );
    let _ = io::stdout().flush();

    drain_pending_events();
    // Terminal emulators can still flush one last report after the disable CSI.
    std::thread::sleep(Duration::from_millis(30));
    drain_pending_events();

    let _ = execute!(io::stdout(), LeaveAlternateScreen, ResetColor, Show);
    let _ = io::stdout().flush();
    write_tty_bytes(b"\x1b[?1049l\x1b[0m\x1b[?25h");
    let _ = disable_raw_mode();

    // Drop any bytes still sitting in the kernel tty buffer after cooked mode returns.
    drain_os_tty_input();
}

fn drain_pending_events() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

fn write_tty_bytes(bytes: &[u8]) {
    #[cfg(unix)]
    {
        use std::io::Write;
        if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            let _ = tty.write_all(bytes);
            let _ = tty.flush();
            return;
        }
    }
    let _ = io::stdout().write_all(bytes);
    let _ = io::stdout().flush();
}

fn drain_os_tty_input() {
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::fd::AsRawFd;

        let Ok(mut tty) = std::fs::OpenOptions::new().read(true).write(false).open("/dev/tty")
        else {
            return;
        };
        let fd = tty.as_raw_fd();
        // SAFETY: setting O_NONBLOCK on our /dev/tty fd so read returns immediately.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        let mut buf = [0u8; 4096];
        for _ in 0..16 {
            match tty.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

/// Route tracing to a log file — never to the tty (that garbles the ratatui frame).
fn init_tui_tracing() {
    let log_path = {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))
            })
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let dir = base.join("rusticlens");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("tui.log")
    };

    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(_) => return,
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .try_init();
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
) -> io::Result<()> {
    let mut last_click: Option<(u16, u16, Instant)> = None;

    loop {
        app.kickoff_connect_if_needed();
        app.poll_connect().await;

        if let Some(req) = app.pending_external.take() {
            handle_external(terminal, app, req).await?;
        }

        app.poll_log_lines();

        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if handle_key(app, key).await {
                        break;
                    }
                }
                Event::Key(_) => {}
                Event::Resize(_, _) => {
                    // Drop leftover glyphs from the previous geometry (classic "doubled" UI).
                    terminal.clear()?;
                }
                Event::Mouse(mouse) if app.log_view_open() && !app.log_search_active() => {
                    handle_log_mouse(app, mouse, &mut last_click);
                }
                Event::Mouse(mouse)
                    if app.is_connected()
                        && !app.log_view_open()
                        && !app.overlay_open()
                        && app.view_mode == ViewMode::Browser =>
                {
                    handle_browser_mouse(app, mouse, &mut last_click);
                }
                Event::Mouse(_) => {}
                _ => {}
            }
        }

        if app.is_connected() && !app.log_view_open() {
            app.poll_snapshots().await;
        }
    }
    Ok(())
}

async fn handle_external(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
    req: ExternalRequest,
) -> io::Result<()> {
    match req {
        ExternalRequest::ApplyYaml => {
            let yaml = suspend_for_editor(terminal, "# Enter YAML to apply\n")?;
            if yaml.trim().is_empty() || yaml.trim() == "# Enter YAML to apply" {
                app.status_message = "Apply cancelled.".into();
                return Ok(());
            }
            if let Some(manager) = app.manager.as_ref() {
                match manager.apply_yaml(&yaml).await {
                    Ok(names) => {
                        app.status_message = format!("Applied: {}", names.join(", "));
                        app.error_message = None;
                        app.refresh().await;
                    }
                    Err(err) => app.error_message = Some(err.user_message()),
                }
            }
        }
        ExternalRequest::EditYaml { name } => {
            let Some(manager) = app.manager.as_ref() else {
                return Ok(());
            };
            let initial = match manager.resource_yaml(app.active_kind, &name).await {
                Ok(yaml) => yaml,
                Err(err) => {
                    app.error_message = Some(err.user_message());
                    return Ok(());
                }
            };
            let yaml = suspend_for_editor(terminal, &initial)?;
            if yaml.trim().is_empty() {
                app.status_message = "Edit cancelled.".into();
                return Ok(());
            }
            if let Some(manager) = app.manager.as_ref() {
                match manager.apply_yaml(&yaml).await {
                    Ok(names) => {
                        app.status_message = format!("Applied edit: {}", names.join(", "));
                        app.error_message = None;
                        app.refresh().await;
                    }
                    Err(err) => app.error_message = Some(err.user_message()),
                }
            }
        }
        ExternalRequest::ExecShell { name, container } => {
            let Some(manager) = app.manager.as_ref() else {
                return Ok(());
            };
            let context = manager.context().to_string();
            let namespace = manager.namespace().to_string();
            let name = name.clone();
            let container = container.clone();

            // Capture result so failures are visible after the alternate screen returns.
            let mut result: Result<(), String> = Ok(());
            suspend_for_command(terminal, || {
                result =
                    run_kubectl_exec_in_place(&context, &namespace, &name, container.as_deref());
            })?;

            match result {
                Ok(()) => {
                    app.status_message = format!("Returned from exec on {name}");
                    app.error_message = None;
                }
                Err(err) => {
                    app.error_message = Some(err);
                    app.status_message = format!("Exec failed for {name}");
                }
            }
        }
    }
    Ok(())
}

fn suspend_for_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    initial: &str,
) -> io::Result<String> {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("rusticlens-edit-{stamp}.yaml"));
    fs::write(&path, initial)?;

    suspend_for_command(terminal, || {
        let status = Command::new(&editor).arg(&path).status();
        if let Err(err) = status {
            let _ = writeln!(io::stderr(), "editor failed: {err}");
        }
    })?;

    let content = fs::read_to_string(&path).unwrap_or_default();
    let _ = fs::remove_file(&path);
    Ok(content)
}

fn suspend_for_command<F>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    f: F,
) -> io::Result<()>
where
    F: FnOnce(),
{
    // Same order as restore_terminal: kill mouse tracking before leaving raw mode.
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange
    )?;
    let _ = terminal.backend_mut().flush();
    drain_pending_events();
    std::thread::sleep(Duration::from_millis(20));
    drain_pending_events();

    execute!(terminal.backend_mut(), LeaveAlternateScreen, Show)?;
    disable_raw_mode()?;
    drain_os_tty_input();

    struct ReenterTui<'a> {
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    }
    impl Drop for ReenterTui<'_> {
        fn drop(&mut self) {
            drain_os_tty_input();
            let _ = enable_raw_mode();
            let _ = execute!(
                self.terminal.backend_mut(),
                EnterAlternateScreen,
                EnableMouseCapture
            );
            let _ = self.terminal.hide_cursor();
            let _ = self.terminal.clear();
        }
    }

    // Always re-enter TUI modes, even if the child panics or leaves termios messy.
    let _reenter = ReenterTui { terminal };
    f();
    Ok(())
}

/// Run `kubectl exec -it` in the current TTY using the active rusticlens context.
fn run_kubectl_exec_in_place(
    context: &str,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<(), String> {
    let mut args = vec![
        "exec".to_string(),
        "-it".to_string(),
        "--context".to_string(),
        context.to_string(),
        "-n".to_string(),
        namespace.to_string(),
        pod_name.to_string(),
    ];
    if let Some(c) = container {
        args.push("-c".to_string());
        args.push(c.to_string());
    }
    // Freelens/Lens: sh -c "clear; (bash || ash || sh)"
    args.push("--".to_string());
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(rl_core::ops::POD_SHELL_WRAPPER.to_string());

    let cmdline = format!("kubectl {}", args.join(" "));
    let _ = writeln!(io::stdout(), "\nrusticlens: {cmdline}\n");

    match Command::new("kubectl").args(&args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            let msg = format!(
                "{cmdline} exited with {status}. If the image has no shell (bash/ash/sh), exec cannot work; otherwise check RBAC / container selection."
            );
            let _ = writeln!(io::stderr(), "{msg}");
            Err(msg)
        }
        Err(err) => Err(format!("failed to run kubectl (is it on PATH?): {err}")),
    }
}

async fn handle_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    if app.log_view_open() {
        return handle_log_key(app, key);
    }

    if app.detail_search_active() {
        return handle_detail_search_key(app, key);
    }

    if app.search_mode {
        return handle_search_key(app, key);
    }

    if app.overlay_open() {
        return handle_overlay_key(app, key).await;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('d') if app.is_connected() => {
                app.prompt_delete();
                return false;
            }
            KeyCode::Char('t') if app.is_connected() => {
                app.add_cluster_tab();
                return false;
            }
            KeyCode::Char('w') if app.is_connected() => {
                app.close_cluster_tab().await;
                return false;
            }
            KeyCode::Char('f') if app.is_connected() => {
                app.enter_detail_search();
                return false;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char('?') => app.open_help(),
        KeyCode::Char(',') => app.open_settings(),
        KeyCode::Char('t') if app.is_connected() => app.toggle_theme(),
        KeyCode::Char('o') if app.is_connected() => app.toggle_overview().await,
        KeyCode::Char('[') if app.is_connected() => app.cycle_cluster_tab(-1).await,
        KeyCode::Char(']') if app.is_connected() => app.cycle_cluster_tab(1).await,
        KeyCode::Char('m') if app.is_connected() => app.open_action_menu(),
        KeyCode::Char('s') if app.is_connected() => app.prompt_scale(),
        KeyCode::Char('R') if app.is_connected() => app.restart_selection().await,
        KeyCode::Char('a') if app.is_connected() => app.request_apply_yaml(),
        KeyCode::Char('E') if app.is_connected() => app.request_edit_yaml(),
        KeyCode::Char('p') if app.is_connected() => app.prompt_port_forward(),
        KeyCode::Char('P') if app.is_connected() => app.open_port_forward_list(),
        KeyCode::Char('e') if app.is_connected() => app.request_exec_shell().await,
        KeyCode::Char('f') if app.is_connected() => app.toggle_favorite_selection(),
        KeyCode::Char('F') if app.is_connected() => app.open_favorites_picker(),
        KeyCode::Char('r') if app.is_connected() => {
            if app.view_mode == ViewMode::Overview {
                app.refresh_dashboard().await;
            } else {
                app.refresh().await;
            }
        }
        KeyCode::Char('r') if app.needs_connect() => app.retry_connect().await,
        KeyCode::Char('d') if app.is_connected() => app.load_detail().await,
        KeyCode::Char('1') if app.is_connected() => app.set_detail_tab(DetailTab::Describe),
        KeyCode::Char('2') if app.is_connected() => app.set_detail_tab(DetailTab::Events),
        KeyCode::Char('3') if app.is_connected() => app.set_detail_tab(DetailTab::Metrics),
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Char('j') => app.move_selection(1),
        KeyCode::Left | KeyCode::Char('h')
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && app.is_connected()
                && app.focus == app::FocusPane::Detail =>
        {
            app.scroll_detail_x_by(-1);
        }
        KeyCode::Right | KeyCode::Char('l')
            if key.modifiers.contains(KeyModifiers::SHIFT)
                && app.is_connected()
                && app.focus == app::FocusPane::Detail =>
        {
            app.scroll_detail_x_by(1);
        }
        KeyCode::Left | KeyCode::Char('h') => app.focus_left(),
        KeyCode::Right | KeyCode::Char('l') => app.focus_right(),
        KeyCode::Tab => {
            if app.is_connected() {
                app.next_kind().await;
            }
        }
        KeyCode::BackTab => {
            if app.is_connected() {
                app.prev_kind().await;
            }
        }
        KeyCode::Enter if app.is_connected() => {
            if app.focus == app::FocusPane::Sidebar {
                app.activate_sidebar_selection().await;
            } else {
                app.load_detail().await;
            }
        }
        KeyCode::Char('c') if app.is_connected() => app.open_context_picker().await,
        KeyCode::Char('n')
            if app.is_connected()
                && app.focus == app::FocusPane::Detail
                && !app.detail_search_query.is_empty() =>
        {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                app.detail_prev_match();
            } else {
                app.detail_next_match();
            }
        }
        KeyCode::Char('N')
            if app.is_connected()
                && app.focus == app::FocusPane::Detail
                && !app.detail_search_query.is_empty() =>
        {
            app.detail_prev_match();
        }
        KeyCode::Char('n') if app.is_connected() => app.open_namespace_picker().await,
        KeyCode::Char('L') if app.is_connected() => {
            if app.active_kind == rl_core::ResourceKind::Service {
                app.start_logs_for_service_selection().await;
            } else {
                app.start_logs_for_selection().await;
            }
        }
        KeyCode::Char('/') if app.is_connected() && app.focus == app::FocusPane::Detail => {
            app.enter_detail_search();
        }
        KeyCode::Char('/') if app.is_connected() => {
            app.enter_search_mode();
        }
        KeyCode::Char('y')
            if app.is_connected() && app.active_kind == rl_core::ResourceKind::Crd =>
        {
            app.next_crd_target().await;
        }
        KeyCode::Char('y') if app.is_connected() && !app.log_view_open() => {
            app.yank_selected_resource_name();
        }
        _ => {}
    }
    false
}

fn handle_log_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    if app.log_search_active() {
        return handle_log_search_key(app, key);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.close_log_view(),
        KeyCode::Char('/') => app.enter_log_search(),
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::SHIFT) => app.log_prev_match(),
        KeyCode::Char('n') => app.log_next_match(),
        KeyCode::Up | KeyCode::Char('k') => app.log_scroll(-1),
        KeyCode::Down | KeyCode::Char('j') => app.log_scroll(1),
        KeyCode::PageUp => app.log_scroll(-10),
        KeyCode::PageDown => app.log_scroll(10),
        KeyCode::Char('f') => app.log_toggle_follow(),
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.log_follow_bottom()
        }
        KeyCode::Char('G') => app.log_follow_bottom(),
        KeyCode::Char('g') => app.log_scroll_top(),
        KeyCode::End => app.log_follow_bottom(),
        KeyCode::Home => app.log_scroll_top(),
        KeyCode::Char('y') => {
            if let Some(row) = app
                .log_layout
                .map(|layout| layout.scroll)
                .or_else(|| app.log_view.as_ref().map(|log| log.scroll))
            {
                app.yank_log_line_at_wrapped_row(row);
            }
        }
        _ => {}
    }
    false
}

fn handle_log_mouse(
    app: &mut TuiApp,
    mouse: MouseEvent,
    last_click: &mut Option<(u16, u16, Instant)>,
) {
    match mouse.kind {
        MouseEventKind::ScrollUp => app.log_scroll(-3),
        MouseEventKind::ScrollDown => app.log_scroll(3),
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(wrapped_row) = app.wrapped_row_at_terminal_pos(mouse.row, mouse.column) else {
                return;
            };
            let now = Instant::now();
            let is_double = last_click.is_some_and(|(col, row, t)| {
                col == mouse.column
                    && row == mouse.row
                    && now.duration_since(t) < Duration::from_millis(400)
            });
            *last_click = Some((mouse.column, mouse.row, now));
            if is_double {
                app.yank_log_line_at_wrapped_row(wrapped_row);
            }
        }
        _ => {}
    }
}

fn handle_detail_mouse(app: &mut TuiApp, mouse: MouseEvent) {
    // Scroll whenever the pointer is over the detail body (trackpad / wheel).
    if matches!(
        mouse.kind,
        MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
    ) {
        if !app.detail_contains_pos(mouse.row, mouse.column) {
            return;
        }
        app.focus = app::FocusPane::Detail;
        let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
        match mouse.kind {
            MouseEventKind::ScrollUp if shift => app.scroll_detail_x_by(-1),
            MouseEventKind::ScrollDown if shift => app.scroll_detail_x_by(1),
            MouseEventKind::ScrollLeft => app.scroll_detail_x_by(-1),
            MouseEventKind::ScrollRight => app.scroll_detail_x_by(1),
            MouseEventKind::ScrollUp => app.scroll_detail_by(-3),
            MouseEventKind::ScrollDown => app.scroll_detail_by(3),
            _ => {}
        }
        return;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let Some((line, col)) = app.detail_pos_at_terminal(mouse.row, mouse.column) else {
                return;
            };
            app.begin_detail_selection(line, col);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((line, col)) = app.detail_pos_at_terminal(mouse.row, mouse.column) {
                app.update_detail_selection(line, col);
            }
        }
        MouseEventKind::Up(MouseButton::Left)
            if app
                .detail_selection
                .is_some_and(|s| s.dragging || app.selected_detail_text().is_some()) =>
        {
            app.finish_detail_selection();
        }
        _ => {}
    }
}

fn handle_browser_mouse(
    app: &mut TuiApp,
    mouse: MouseEvent,
    last_click: &mut Option<(u16, u16, Instant)>,
) {
    let table_dragging = app.table_selection.is_some_and(|s| s.dragging);
    let detail_dragging = app.detail_selection.is_some_and(|s| s.dragging);

    if app.table_contains_pos(mouse.row, mouse.column)
        || (table_dragging
            && matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            ))
    {
        handle_table_mouse(app, mouse, last_click);
        return;
    }
    if app.detail_contains_pos(mouse.row, mouse.column)
        || (detail_dragging
            && matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            ))
    {
        handle_detail_mouse(app, mouse);
    }
}

fn handle_table_mouse(
    app: &mut TuiApp,
    mouse: MouseEvent,
    last_click: &mut Option<(u16, u16, Instant)>,
) {
    if matches!(
        mouse.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) {
        let visible = app
            .table_layout
            .map(|l| l.area.height.max(1) as usize)
            .unwrap_or(10);
        // Always scroll the list when the wheel fires over the middle panel.
        match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_table_by(-5, visible),
            MouseEventKind::ScrollDown => app.scroll_table_by(5, visible),
            _ => {}
        }
        return;
    }

    // Text selection / row click only on the data body.
    if !app.table_body_contains_pos(mouse.row, mouse.column)
        && !app.table_selection.is_some_and(|s| s.dragging)
    {
        return;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let Some((line, col)) = app.table_pos_at_terminal(mouse.row, mouse.column) else {
                return;
            };
            let now = Instant::now();
            let is_double = last_click.is_some_and(|(c, r, t)| {
                c == mouse.column
                    && r == mouse.row
                    && now.duration_since(t) < Duration::from_millis(400)
            });
            *last_click = Some((mouse.column, mouse.row, now));
            if is_double {
                if line < app.table_lines.len() {
                    app.selected = line;
                }
                app.yank_selected_resource_name();
                app.table_selection = None;
                return;
            }
            app.begin_table_selection(line, col);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((line, col)) = app.table_pos_at_terminal(mouse.row, mouse.column) {
                app.update_table_selection(line, col);
            }
        }
        MouseEventKind::Up(MouseButton::Left)
            if app
                .table_selection
                .is_some_and(|s| s.dragging || app.selected_table_text().is_some()) =>
        {
            app.finish_table_selection();
        }
        _ => {}
    }
}

fn handle_log_search_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            if app
                .log_view
                .as_ref()
                .is_some_and(|l| l.search_query.is_empty())
            {
                app.exit_log_search();
            } else {
                app.clear_log_search();
            }
        }
        KeyCode::Enter => app.exit_log_search(),
        KeyCode::Backspace => app.log_search_backspace(),
        KeyCode::Char('/') => app.exit_log_search(),
        KeyCode::Char(ch) if !ch.is_control() => app.log_search_push_char(ch),
        _ => {}
    }
    false
}

fn handle_search_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            if app.table_filter.is_empty() {
                app.exit_search_mode();
            } else {
                app.clear_search();
            }
        }
        KeyCode::Enter => app.exit_search_mode(),
        KeyCode::Backspace => app.search_backspace(),
        KeyCode::Char('/') => app.exit_search_mode(),
        KeyCode::Char(ch) if !ch.is_control() => app.search_push_char(ch),
        _ => {}
    }
    false
}

fn handle_detail_search_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            if app.detail_search_query.is_empty() {
                app.exit_detail_search();
            } else {
                app.clear_detail_search();
            }
        }
        KeyCode::Enter => app.exit_detail_search(),
        KeyCode::Backspace => app.detail_search_backspace(),
        KeyCode::Char('/') => app.exit_detail_search(),
        KeyCode::Char(ch) if !ch.is_control() => app.detail_search_push_char(ch),
        _ => {}
    }
    false
}

async fn handle_overlay_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    if matches!(
        app.overlay,
        Some(Overlay::Settings {
            cursor: SettingsCursor::ExtraKubeconfigList,
            ..
        })
    ) && matches!(key.code, KeyCode::Char('d') | KeyCode::Delete)
    {
        if let Some(Overlay::Settings { path_selected, .. }) = app.overlay.clone() {
            if path_selected < app.extra_kubeconfig_paths.len() {
                let removed = app.extra_kubeconfig_paths.remove(path_selected);
                app.persist_ui_settings();
                app.status_message = format!("Removed kubeconfig: {removed}");
            }
        }
        return false;
    }

    match key.code {
        KeyCode::Esc => app.close_overlay(),
        KeyCode::Enter => app.picker_confirm().await,
        KeyCode::Up | KeyCode::Char('k') => app.picker_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.picker_move(1),
        KeyCode::Backspace => app.picker_backspace(),
        KeyCode::Char(ch) if !ch.is_control() => app.picker_push_char(ch),
        _ => {}
    }
    false
}
