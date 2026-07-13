mod app;
mod ui;

use std::io::{self, stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_subscriber::EnvFilter;

use app::{DetailTab, TuiApp};

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new();
    let result = run(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TuiApp,
) -> io::Result<()> {
    let mut last_click: Option<(u16, u16, Instant)> = None;

    loop {
        if app.needs_connect() && !app.connect_attempted() {
            app.connect().await;
        }

        app.poll_log_lines();

        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(app, key).await {
                        break;
                    }
                }
                Event::Mouse(mouse) if app.log_view_open() && !app.log_search_active() => {
                    handle_log_mouse(app, mouse, &mut last_click);
                }
                Event::Mouse(_) => {}
                _ => {}
            }
        }

        if app.is_connected() {
            app.poll_snapshots().await;
        }
    }
    Ok(())
}

async fn handle_key(app: &mut TuiApp, key: KeyEvent) -> bool {
    if app.log_view_open() {
        return handle_log_key(app, key);
    }

    if app.search_mode {
        return handle_search_key(app, key);
    }

    if app.overlay_open() {
        return handle_overlay_key(app, key).await;
    }

    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('r') if app.is_connected() => app.refresh().await,
        KeyCode::Char('r') if app.needs_connect() => app.retry_connect().await,
        KeyCode::Char('d') if app.is_connected() => app.load_detail().await,
        KeyCode::Char('1') if app.is_connected() => app.set_detail_tab(DetailTab::Describe),
        KeyCode::Char('2') if app.is_connected() => app.set_detail_tab(DetailTab::Events),
        KeyCode::Char('3') if app.is_connected() => app.set_detail_tab(DetailTab::Metrics),
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Char('j') => app.move_selection(1),
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
        KeyCode::Char('n') if app.is_connected() => app.open_namespace_picker().await,
        KeyCode::Char('L') if app.is_connected() => app.start_logs_for_selection().await,
        KeyCode::Char('/')
            if app.is_connected() && app.active_kind == rl_core::ResourceKind::Pod =>
        {
            app.enter_search_mode();
        }
        KeyCode::Char('y')
            if app.is_connected() && app.active_kind == rl_core::ResourceKind::Crd =>
        {
            app.next_crd_target().await;
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

async fn handle_overlay_key(app: &mut TuiApp, key: KeyEvent) -> bool {
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
