mod features;

use std::collections::HashMap;
use std::process::Child;

use ratatui::layout::Rect;

use rl_core::{
    config::context_is_usable, format_events_text, format_metrics_text, load_settings,
    save_settings, ClusterDashboard, ClusterManager, CrdTarget, FavoriteResource,
    PortForwardHandle, ResourceKind, ResourceRow, LOG_BUFFER_MAX_LINES,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::theme::ThemeMode;

pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Describe,
    Events,
    Metrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    Sidebar,
    Table,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Browser,
    Overview,
}

#[derive(Debug, Clone)]
pub struct ListPickerState {
    pub search: String,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum PendingAction {
    Delete,
    Scale,
    Restart,
    TriggerCronJob,
    SuspendCronJob,
    ResumeCronJob,
    StartPortForward,
    ToggleFavorite,
    OpenLogs,
    OpenServiceLogs,
    ExecShell,
    ApplyYaml,
    EditYaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPurpose {
    ScaleReplicas,
    PortForwardLocal,
    PortForwardRemote,
    AddKubeconfigPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCursor {
    NativePortForward,
    Theme,
    AddKubeconfigPath,
    ExtraKubeconfigList,
}

pub enum PortForwardSession {
    Native(PortForwardHandle),
    Kubectl(Child),
}

impl PortForwardSession {
    pub fn stop(self) {
        match self {
            PortForwardSession::Native(handle) => handle.stop(),
            PortForwardSession::Kubectl(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortForwardEntry {
    pub id: u64,
    pub label: String,
    pub local_port: u16,
    pub remote_port: u16,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Overlay {
    Context(ListPickerState),
    Namespace(ListPickerState),
    Container {
        pod_name: String,
        containers: Vec<String>,
        state: ListPickerState,
    },
    Confirm {
        title: String,
        detail: String,
        action: PendingAction,
    },
    Input {
        prompt: String,
        value: String,
        purpose: InputPurpose,
        /// Secondary value for two-step PF (remote port).
        extra: Option<String>,
    },
    ActionMenu {
        items: Vec<crate::actions::ActionItem>,
        selected: usize,
        filter: String,
    },
    Favorites(ListPickerState),
    PortForwardList {
        selected: usize,
    },
    Settings {
        cursor: SettingsCursor,
        path_selected: usize,
    },
    Help,
}

pub struct LogView {
    pub pod_name: String,
    pub container: Option<String>,
    pub lines: Vec<String>,
    /// Cached soft-wrapped display rows (invalidated when width/lines change).
    wrapped_cache: Vec<String>,
    /// Source line index for each entry in `wrapped_cache`.
    wrapped_sources: Vec<usize>,
    wrap_cache_width: usize,
    pub scroll: usize,
    pub follow: bool,
    pub visible_lines: usize,
    pub wrap_width: usize,
    pub search_mode: bool,
    pub search_query: String,
    pub match_rows: Vec<usize>,
    pub match_cursor: usize,
    matches_dirty: bool,
    pub error: Option<String>,
    /// Source line index to highlight after yank (all wrapped segments).
    pub highlight_source: Option<usize>,
    line_rx: mpsc::Receiver<String>,
    err_rx: mpsc::Receiver<String>,
    stream_task: JoinHandle<()>,
}

/// Screen region of the log body — used to map mouse clicks to wrapped rows.
#[derive(Debug, Clone, Copy)]
pub struct LogViewLayout {
    pub area: Rect,
    pub scroll: usize,
}

/// Screen region of the detail body — mouse scroll / selection hit-testing.
#[derive(Debug, Clone, Copy)]
pub struct DetailLayout {
    pub area: Rect,
    pub scroll: usize,
    pub scroll_x: usize,
}

/// Screen region of the resource table.
#[derive(Debug, Clone, Copy)]
pub struct TableLayout {
    /// Full table panel (border + header + body) — used for mouse-wheel hit testing.
    pub panel: Rect,
    /// Data body only — used for drag-select hit testing.
    pub area: Rect,
    pub scroll: usize,
}

/// Text selection inside the detail or table pane (absolute content line/col).
#[derive(Debug, Clone, Copy)]
pub struct TextSelection {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub dragging: bool,
}

impl TextSelection {
    pub fn normalized(&self) -> ((usize, usize), (usize, usize)) {
        let a = (self.start_line, self.start_col);
        let b = (self.end_line, self.end_col);
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    }
}

/// Backward-compatible alias used by detail code.
pub type DetailSelection = TextSelection;

pub struct TuiApp {
    pub connection: ConnectionState,
    pub manager: Option<ClusterManager>,
    pub active_kind: ResourceKind,
    pub sidebar_index: usize,
    pub focus: FocusPane,
    pub view_mode: ViewMode,
    pub rows: Vec<ResourceRow>,
    pub selected: usize,
    pub contexts: Vec<String>,
    pub context_index: usize,
    pub namespaces: Vec<String>,
    pub namespace_index: usize,
    pub detail_tab: DetailTab,
    pub detail_yaml: String,
    pub detail_events: String,
    pub detail_metrics: String,
    pub detail_scroll: u16,
    pub detail_scroll_x: usize,
    pub detail_layout: Option<DetailLayout>,
    pub detail_selection: Option<DetailSelection>,
    /// Find-in-detail (`/` when detail focused, or Ctrl+F).
    pub detail_search_mode: bool,
    pub detail_search_query: String,
    pub detail_match_rows: Vec<usize>,
    pub detail_match_cursor: usize,
    pub table_layout: Option<TableLayout>,
    pub table_selection: Option<TextSelection>,
    /// Display lines for the filtered resource table (for mouse selection / copy).
    pub table_lines: Vec<String>,
    pub table_scroll: usize,
    pub status_message: String,
    pub error_message: Option<String>,
    pub crd_targets: Vec<CrdTarget>,
    pub selected_crd_index: usize,
    pub overlay: Option<Overlay>,
    pub log_view: Option<LogView>,
    pub log_layout: Option<LogViewLayout>,
    pub table_filter: String,
    pub search_mode: bool,
    pub theme_mode: ThemeMode,
    pub cluster_tabs: Vec<String>,
    pub favorites: Vec<FavoriteResource>,
    pub use_native_port_forward: bool,
    pub extra_kubeconfig_paths: Vec<String>,
    pub port_forwards: HashMap<u64, PortForwardSession>,
    pub port_forward_entries: Vec<PortForwardEntry>,
    pub next_port_forward_id: u64,
    pub dashboard: Option<ClusterDashboard>,
    /// Set by action handlers so main loop can suspend TUI for editor/exec.
    pub pending_external: Option<ExternalRequest>,
    connect_attempted: bool,
}

#[derive(Debug, Clone)]
pub enum ExternalRequest {
    ApplyYaml,
    EditYaml {
        name: String,
    },
    ExecShell {
        name: String,
        container: Option<String>,
    },
}

impl TuiApp {
    pub fn connect_attempted(&self) -> bool {
        self.connect_attempted
    }

    pub fn new() -> Self {
        let settings = load_settings();
        let theme_mode = ThemeMode::from_settings();
        let cluster_tabs = if settings.open_cluster_tabs.is_empty() {
            settings
                .last_context
                .clone()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            settings.open_cluster_tabs.clone()
        };
        Self {
            connection: ConnectionState::Disconnected,
            manager: None,
            active_kind: ResourceKind::Pod,
            sidebar_index: 0,
            focus: FocusPane::Table,
            view_mode: ViewMode::Browser,
            rows: Vec::new(),
            selected: 0,
            contexts: Vec::new(),
            context_index: 0,
            namespaces: Vec::new(),
            namespace_index: 0,
            detail_tab: DetailTab::Describe,
            detail_yaml: String::new(),
            detail_events: String::new(),
            detail_metrics: String::new(),
            detail_scroll: 0,
            detail_scroll_x: 0,
            detail_layout: None,
            detail_selection: None,
            detail_search_mode: false,
            detail_search_query: String::new(),
            detail_match_rows: Vec::new(),
            detail_match_cursor: 0,
            table_layout: None,
            table_selection: None,
            table_lines: Vec::new(),
            table_scroll: 0,
            status_message: String::new(),
            error_message: None,
            crd_targets: Vec::new(),
            selected_crd_index: 0,
            overlay: None,
            log_view: None,
            log_layout: None,
            table_filter: String::new(),
            search_mode: false,
            theme_mode,
            cluster_tabs,
            favorites: settings.favorites,
            use_native_port_forward: settings.use_native_port_forward,
            extra_kubeconfig_paths: settings.extra_kubeconfig_paths,
            port_forwards: HashMap::new(),
            port_forward_entries: Vec::new(),
            next_port_forward_id: 1,
            dashboard: None,
            pending_external: None,
            connect_attempted: false,
        }
    }

    pub fn persist_ui_settings(&self) {
        let mut settings = load_settings();
        settings.theme = Some(self.theme_mode.as_str().to_string());
        settings.open_cluster_tabs = self.cluster_tabs.clone();
        settings.favorites = self.favorites.clone();
        settings.use_native_port_forward = self.use_native_port_forward;
        settings.extra_kubeconfig_paths = self.extra_kubeconfig_paths.clone();
        let _ = save_settings(&settings);
    }

    pub fn fire_plugins(&self, context: &str) {
        rl_core::ensure_plugins_dir();
        let mut registry = rl_core::plugins::PluginRegistry::new();
        rl_core::plugin_loader::load_manifest_plugins(&mut registry);
        registry.notify_connected(context);
    }

    pub fn enter_search_mode(&mut self) {
        self.search_mode = true;
        self.focus = FocusPane::Table;
    }

    pub fn exit_search_mode(&mut self) {
        self.search_mode = false;
    }

    pub fn clear_search(&mut self) {
        self.table_filter.clear();
        self.clamp_table_selection();
    }

    pub fn search_push_char(&mut self, ch: char) {
        self.table_filter.push(ch);
        self.selected = 0;
        self.detail_scroll = 0;
        self.clear_detail();
    }

    pub fn search_backspace(&mut self) {
        self.table_filter.pop();
        self.clamp_table_selection();
    }

    pub fn filtered_row_indices(&self) -> Vec<usize> {
        if self.table_filter.is_empty() {
            return (0..self.rows.len()).collect();
        }
        let query = self.table_filter.to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.name.to_lowercase().contains(&query))
            .map(|(idx, _)| idx)
            .collect()
    }

    pub fn selected_row_index(&self) -> Option<usize> {
        self.filtered_row_indices().get(self.selected).copied()
    }

    fn clamp_table_selection(&mut self) {
        let count = self.filtered_row_indices().len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    pub fn log_view_open(&self) -> bool {
        self.log_view.is_some()
    }

    pub fn close_log_view(&mut self) {
        if let Some(log) = self.log_view.take() {
            log.stream_task.abort();
        }
        self.log_layout = None;
        if self.status_message.contains("Polling service pod logs") {
            self.status_message.clear();
        }
    }

    pub fn poll_log_lines(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        // Cap intake per tick so a multi-pod flood cannot freeze the UI loop.
        // Wrap updates are incremental, so a larger batch is fine.
        const MAX_LINES_PER_TICK: usize = 1_024;
        let mut received = 0usize;
        while received < MAX_LINES_PER_TICK {
            match log.line_rx.try_recv() {
                Ok(line) => {
                    log.push_line(line);
                    received += 1;
                }
                Err(_) => break,
            }
        }
        while let Ok(err) = log.err_rx.try_recv() {
            log.error = Some(err);
        }
        if received > 0 {
            log.after_batch_ingest();
        }
    }

    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }

    pub fn close_overlay(&mut self) {
        self.overlay = None;
    }

    pub fn needs_connect(&self) -> bool {
        matches!(
            self.connection,
            ConnectionState::Disconnected | ConnectionState::Failed(_)
        )
    }

    pub async fn connect(&mut self) {
        if matches!(
            self.connection,
            ConnectionState::Connecting | ConnectionState::Connected
        ) {
            return;
        }
        self.connection = ConnectionState::Connecting;
        self.connect_attempted = true;
        self.error_message = None;
        self.status_message = "Connecting...".into();

        match ClusterManager::connect_default(self.active_kind).await {
            Ok(mut manager) => {
                let namespaces = manager.list_namespaces().await.unwrap_or_default();
                let namespace_index = namespaces
                    .iter()
                    .position(|n| n == manager.namespace())
                    .unwrap_or(0);
                self.crd_targets = manager.crd_targets().to_vec();
                if self.active_kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
                    let target = self.crd_targets[self
                        .selected_crd_index
                        .min(self.crd_targets.len().saturating_sub(1))]
                    .clone();
                    manager.set_selected_crd(Some(target));
                }
                self.namespaces = namespaces;
                self.namespace_index = namespace_index;
                let ctx = manager.context().to_string();
                if !self.cluster_tabs.iter().any(|t| t == &ctx) {
                    self.cluster_tabs.push(ctx.clone());
                }
                self.manager = Some(manager);
                self.sync_context_list().await;
                self.connection = ConnectionState::Connected;
                self.status_message = "Connected".into();
                self.fire_plugins(&ctx);
                self.persist_ui_settings();
                self.refresh().await;
            }
            Err(err) => {
                let msg = err.user_message();
                self.connection = ConnectionState::Failed(msg.clone());
                self.manager = None;
                self.status_message.clear();
                self.error_message = Some(msg);
            }
        }
    }

    pub async fn retry_connect(&mut self) {
        self.connection = ConnectionState::Disconnected;
        self.connect().await;
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, ConnectionState::Connected)
    }

    pub async fn refresh(&mut self) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };

        let kind = self.active_kind;
        if kind == ResourceKind::Crd {
            if self.crd_targets.is_empty() {
                self.rows.clear();
                self.status_message = "No CRDs in cluster".into();
                return;
            }
            let idx = self
                .selected_crd_index
                .min(self.crd_targets.len().saturating_sub(1));
            let target = self.crd_targets[idx].clone();
            manager.set_selected_crd(Some(target));
        }

        let status = match manager.list_rows(kind).await {
            Ok(rows) => {
                self.rows = rows;
                self.error_message = None;
                format!(
                    "{} / {} — {} items",
                    manager.context(),
                    manager.namespace(),
                    self.rows.len()
                )
            }
            Err(err) => {
                self.rows.clear();
                self.error_message = Some(err.user_message());
                String::new()
            }
        };

        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.clamp_table_selection();

        if !status.is_empty() {
            self.status_message = status;
        }
    }

    pub async fn poll_snapshots(&mut self) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let kind = self.active_kind;
        if matches!(kind, ResourceKind::HelmRelease | ResourceKind::Crd) {
            return;
        }
        let snapshot = manager.snapshot(kind);
        if snapshot.rows.len() != self.rows.len()
            || snapshot
                .rows
                .iter()
                .zip(self.rows.iter())
                .any(|(a, b)| a.name != b.name || a.status != b.status)
        {
            self.rows = snapshot.rows;
            if self.selected >= self.rows.len() {
                self.selected = self.rows.len().saturating_sub(1);
            }
            self.clamp_table_selection();
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        match self.focus {
            FocusPane::Sidebar => self.move_sidebar(delta),
            FocusPane::Table => self.move_table_selection(delta),
            FocusPane::Detail => self.scroll_detail(delta),
        }
    }

    fn move_table_selection(&mut self, delta: i32) {
        let indices = self.filtered_row_indices();
        if indices.is_empty() {
            return;
        }
        let next = self.selected as i32 + delta;
        self.selected = next.clamp(0, indices.len() as i32 - 1) as usize;
        self.detail_scroll = 0;
        let visible = self
            .table_layout
            .map(|l| l.area.height.max(1) as usize)
            .unwrap_or(10);
        self.ensure_table_selection_visible(visible);
    }

    /// Keep `selected` within the visible table body (`visible_rows` data rows).
    pub fn ensure_table_selection_visible(&mut self, visible_rows: usize) {
        let visible = visible_rows.max(1);
        if self.selected < self.table_scroll {
            self.table_scroll = self.selected;
        } else if self.selected >= self.table_scroll.saturating_add(visible) {
            self.table_scroll = self.selected.saturating_sub(visible - 1);
        }
        let max_scroll = self.table_lines.len().saturating_sub(visible);
        self.table_scroll = self.table_scroll.min(max_scroll);
    }

    pub fn scroll_table_by(&mut self, delta: i32, visible_rows: usize) {
        let visible = visible_rows.max(1);
        let max_scroll = self.table_lines.len().saturating_sub(visible);
        if max_scroll == 0 {
            return;
        }
        let next = self.table_scroll as i32 + delta;
        self.table_scroll = next.clamp(0, max_scroll as i32) as usize;
        // Keep the highlight inside the new viewport so the next paint stays put.
        if self.selected < self.table_scroll {
            self.selected = self.table_scroll;
        } else if self.selected >= self.table_scroll.saturating_add(visible) {
            self.selected = self.table_scroll.saturating_add(visible - 1);
        }
        let last = self.table_lines.len().saturating_sub(1);
        self.selected = self.selected.min(last);
        self.focus = FocusPane::Table;
    }

    pub fn table_contains_pos(&self, row: u16, column: u16) -> bool {
        let Some(layout) = self.table_layout else {
            return false;
        };
        // Wheel / hover uses the whole middle panel so scrolling works on header too.
        let area = layout.panel;
        column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    pub fn table_body_contains_pos(&self, row: u16, column: u16) -> bool {
        let Some(layout) = self.table_layout else {
            return false;
        };
        let area = layout.area;
        column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    pub fn table_pos_at_terminal(&self, row: u16, column: u16) -> Option<(usize, usize)> {
        let layout = self.table_layout?;
        let area = layout.area;
        if column < area.x
            || column >= area.x.saturating_add(area.width)
            || row < area.y
            || row >= area.y.saturating_add(area.height)
        {
            return None;
        }
        let local_row = (row - area.y) as usize;
        let local_col = (column - area.x) as usize;
        let line = layout.scroll.saturating_add(local_row);
        if self.table_lines.is_empty() {
            return None;
        }
        let line = line.min(self.table_lines.len().saturating_sub(1));
        let text = self.table_lines.get(line).map(|s| s.as_str()).unwrap_or("");
        let col = local_col.min(text.chars().count());
        Some((line, col))
    }

    pub fn begin_table_selection(&mut self, line: usize, col: usize) {
        self.focus = FocusPane::Table;
        if line < self.table_lines.len() {
            self.selected = line;
        }
        self.detail_selection = None;
        self.table_selection = Some(TextSelection {
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col,
            dragging: true,
        });
    }

    pub fn update_table_selection(&mut self, line: usize, col: usize) {
        let Some(sel) = self.table_selection.as_mut() else {
            return;
        };
        if !sel.dragging {
            return;
        }
        sel.end_line = line;
        sel.end_col = col;
        if line < self.table_lines.len() {
            self.selected = line;
        }
    }

    pub fn finish_table_selection(&mut self) {
        let Some(sel) = self.table_selection.as_mut() else {
            return;
        };
        sel.dragging = false;
        let ((sl, sc), (el, ec)) = sel.normalized();
        if sl == el && sc == ec {
            // Click without drag: keep row selected, clear text selection.
            self.table_selection = None;
            return;
        }
        if let Some(text) = self.selected_table_text() {
            match crate::clipboard::copy_text(&text) {
                Ok(()) => {
                    let preview = if text.len() > 40 {
                        format!("{}…", text.chars().take(40).collect::<String>())
                    } else {
                        text
                    };
                    self.status_message = format!("Copied selection: {preview}");
                    self.error_message = None;
                }
                Err(err) => {
                    self.error_message = Some(format!("Clipboard unavailable: {err}"));
                }
            }
        }
    }

    pub fn selected_table_text(&self) -> Option<String> {
        let sel = self.table_selection?;
        let ((sl, sc), (el, ec)) = sel.normalized();
        if self.table_lines.is_empty() || sl >= self.table_lines.len() {
            return None;
        }
        let el = el.min(self.table_lines.len().saturating_sub(1));
        if sl == el {
            let line = &self.table_lines[sl];
            let start = sc.min(line.chars().count());
            let end = ec.min(line.chars().count()).max(start);
            return Some(line.chars().skip(start).take(end - start).collect());
        }
        let mut out = String::new();
        let first = &self.table_lines[sl];
        let start = sc.min(first.chars().count());
        out.push_str(&first.chars().skip(start).collect::<String>());
        out.push('\n');
        for line in &self.table_lines[sl + 1..el] {
            out.push_str(line);
            out.push('\n');
        }
        let last = &self.table_lines[el];
        let end = ec.min(last.chars().count());
        out.push_str(&last.chars().take(end).collect::<String>());
        Some(out)
    }

    /// Copy the name of the currently selected resource.
    pub fn yank_selected_resource_name(&mut self) {
        let Some(idx) = self.selected_row_index() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let Some(row) = self.rows.get(idx) else {
            return;
        };
        match crate::clipboard::copy_text(&row.name) {
            Ok(()) => {
                self.status_message = format!("Copied name: {}", row.name);
                self.error_message = None;
            }
            Err(err) => {
                self.error_message = Some(format!("Clipboard unavailable: {err}"));
            }
        }
    }

    fn move_sidebar(&mut self, delta: i32) {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        let current = self.sidebar_index.min(kinds.len().saturating_sub(1));
        let next = (current as i32 + delta).clamp(0, kinds.len() as i32 - 1) as usize;
        self.sidebar_index = next;
    }

    fn scroll_detail(&mut self, delta: i32) {
        let content = self.detail_content();
        let max_scroll = content.lines().count().saturating_sub(1) as i32;
        let next = self.detail_scroll as i32 + delta;
        self.detail_scroll = next.clamp(0, max_scroll) as u16;
    }

    pub fn scroll_detail_x_by(&mut self, delta: i32) {
        let width = self
            .detail_layout
            .map(|l| (l.area.width as usize).max(1))
            .unwrap_or(40);
        let max_x = self.detail_max_scroll_x(width);
        let next = self.detail_scroll_x as i32 + delta;
        self.detail_scroll_x = next.clamp(0, max_x as i32) as usize;
    }

    pub fn detail_max_scroll_x(&self, viewport_width: usize) -> usize {
        let max_len = self
            .detail_content()
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        max_len.saturating_sub(viewport_width.max(1))
    }

    pub fn focus_left(&mut self) {
        self.focus = match self.focus {
            FocusPane::Detail => FocusPane::Table,
            FocusPane::Table => FocusPane::Sidebar,
            FocusPane::Sidebar => FocusPane::Sidebar,
        };
    }

    pub fn focus_right(&mut self) {
        self.focus = match self.focus {
            FocusPane::Sidebar => FocusPane::Table,
            FocusPane::Table => FocusPane::Detail,
            FocusPane::Detail => FocusPane::Detail,
        };
    }

    pub async fn activate_sidebar_selection(&mut self) {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        if let Some(kind) = kinds.get(self.sidebar_index) {
            self.set_kind(*kind).await;
        }
    }

    pub async fn next_kind(&mut self) {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        let idx = kinds
            .iter()
            .position(|k| *k == self.active_kind)
            .unwrap_or(0);
        let next = (idx + 1) % kinds.len();
        self.set_kind(kinds[next]).await;
    }

    pub async fn prev_kind(&mut self) {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        let idx = kinds
            .iter()
            .position(|k| *k == self.active_kind)
            .unwrap_or(0);
        let prev = if idx == 0 { kinds.len() - 1 } else { idx - 1 };
        self.set_kind(kinds[prev]).await;
    }

    async fn set_kind(&mut self, kind: ResourceKind) {
        self.active_kind = kind;
        self.sidebar_index = crate::ui::kind_sidebar_index(kind);
        self.selected = 0;
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.table_filter.clear();
        self.search_mode = false;
        self.table_scroll = 0;
        self.table_selection = None;
        self.clear_detail();

        if let Some(manager) = self.manager.as_mut() {
            if let Err(err) = manager.set_active_kind(kind).await {
                self.error_message = Some(err.user_message());
            }
        }
        self.refresh().await;
    }

    async fn sync_context_list(&mut self) {
        if let Ok(contexts) = ClusterManager::list_contexts().await {
            self.contexts = contexts;
            if let Some(manager) = &self.manager {
                self.context_index = self
                    .contexts
                    .iter()
                    .position(|c| c == manager.context())
                    .unwrap_or(0);
            }
        }
    }

    pub async fn open_context_picker(&mut self) {
        self.sync_context_list().await;
        let selected = self
            .filtered_context_indices("")
            .iter()
            .position(|idx| *idx == self.context_index)
            .unwrap_or(0);
        self.overlay = Some(Overlay::Context(ListPickerState {
            search: String::new(),
            selected,
        }));
    }

    pub async fn open_namespace_picker(&mut self) {
        if let Some(manager) = self.manager.as_mut() {
            self.namespaces = manager.list_namespaces().await.unwrap_or_default();
            self.namespace_index = self
                .namespaces
                .iter()
                .position(|n| n == manager.namespace())
                .unwrap_or(0);
        }
        let selected = self
            .filtered_namespace_indices("")
            .iter()
            .position(|idx| *idx == self.namespace_index)
            .unwrap_or(0);
        self.overlay = Some(Overlay::Namespace(ListPickerState {
            search: String::new(),
            selected,
        }));
    }

    pub fn picker_indices(&self) -> Vec<usize> {
        match &self.overlay {
            Some(Overlay::Context(state)) => self.filtered_context_indices(&state.search),
            Some(Overlay::Namespace(state)) => self.filtered_namespace_indices(&state.search),
            Some(Overlay::Container {
                state, containers, ..
            }) => filter_indices(containers, &state.search),
            Some(Overlay::Favorites(state)) => self.filtered_favorite_indices(&state.search),
            _ => Vec::new(),
        }
    }

    pub fn filtered_favorite_indices(&self, search: &str) -> Vec<usize> {
        let query = search.to_lowercase();
        self.favorites
            .iter()
            .enumerate()
            .filter(|(_, fav)| {
                if query.is_empty() {
                    return true;
                }
                format!("{}/{}/{}", fav.kind, fav.namespace, fav.name)
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    fn filtered_context_indices(&self, search: &str) -> Vec<usize> {
        filter_indices(&self.contexts, search)
    }

    fn filtered_namespace_indices(&self, search: &str) -> Vec<usize> {
        filter_indices(&self.namespaces, search)
    }

    fn picker_filtered_indices(&self, search: &str) -> Vec<usize> {
        match &self.overlay {
            Some(Overlay::Context(_)) => self.filtered_context_indices(search),
            Some(Overlay::Namespace(_)) => self.filtered_namespace_indices(search),
            Some(Overlay::Container { containers, .. }) => filter_indices(containers, search),
            Some(Overlay::Favorites(_)) => self.filtered_favorite_indices(search),
            _ => Vec::new(),
        }
    }

    fn overlay_list_state_mut(&mut self) -> Option<&mut ListPickerState> {
        match &mut self.overlay {
            Some(
                Overlay::Context(state) | Overlay::Namespace(state) | Overlay::Favorites(state),
            ) => Some(state),
            Some(Overlay::Container { state, .. }) => Some(state),
            _ => None,
        }
    }

    fn overlay_search(&self) -> Option<String> {
        match &self.overlay {
            Some(
                Overlay::Context(state) | Overlay::Namespace(state) | Overlay::Favorites(state),
            ) => Some(state.search.clone()),
            Some(Overlay::Container { state, .. }) => Some(state.search.clone()),
            Some(Overlay::ActionMenu { filter, .. }) => Some(filter.clone()),
            _ => None,
        }
    }

    pub fn picker_move(&mut self, delta: i32) {
        if let Some(Overlay::ActionMenu {
            items,
            selected,
            filter,
        }) = &mut self.overlay
        {
            let filtered: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    filter.is_empty() || item.label.to_lowercase().contains(&filter.to_lowercase())
                })
                .map(|(i, _)| i)
                .collect();
            if filtered.is_empty() {
                *selected = 0;
                return;
            }
            let cur = filtered.iter().position(|i| i == selected).unwrap_or(0) as i32;
            let next = (cur + delta).clamp(0, filtered.len() as i32 - 1) as usize;
            *selected = filtered[next];
            return;
        }
        if let Some(Overlay::PortForwardList { selected }) = &mut self.overlay {
            let len = self.port_forward_entries.len();
            if len == 0 {
                *selected = 0;
                return;
            }
            let next = *selected as i32 + delta;
            *selected = next.clamp(0, len as i32 - 1) as usize;
            return;
        }
        if let Some(Overlay::Settings {
            cursor,
            path_selected,
        }) = &mut self.overlay
        {
            let max = 3i32;
            let cur = match cursor {
                SettingsCursor::NativePortForward => 0,
                SettingsCursor::Theme => 1,
                SettingsCursor::AddKubeconfigPath => 2,
                SettingsCursor::ExtraKubeconfigList => 3,
            };
            let next = (cur + delta).clamp(0, max);
            *cursor = match next {
                0 => SettingsCursor::NativePortForward,
                1 => SettingsCursor::Theme,
                2 => SettingsCursor::AddKubeconfigPath,
                _ => SettingsCursor::ExtraKubeconfigList,
            };
            if *cursor == SettingsCursor::ExtraKubeconfigList
                && !self.extra_kubeconfig_paths.is_empty()
            {
                let plen = self.extra_kubeconfig_paths.len();
                let pnext = *path_selected as i32 + delta;
                *path_selected = pnext.clamp(0, plen as i32 - 1) as usize;
            }
            return;
        }
        if matches!(self.overlay, Some(Overlay::Help | Overlay::Confirm { .. })) {
            return;
        }
        if let Some(Overlay::Input { .. }) = &self.overlay {
            return;
        }
        let search = match self.overlay_search() {
            Some(search) => search,
            None => return,
        };
        let indices = self.picker_filtered_indices(&search);
        let Some(state) = self.overlay_list_state_mut() else {
            return;
        };
        if indices.is_empty() {
            state.selected = 0;
            return;
        }
        let next = state.selected as i32 + delta;
        state.selected = next.clamp(0, indices.len() as i32 - 1) as usize;
    }

    pub fn picker_push_char(&mut self, ch: char) {
        if let Some(Overlay::ActionMenu {
            filter,
            selected,
            items,
        }) = &mut self.overlay
        {
            filter.push(ch);
            let filtered: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.label.to_lowercase().contains(&filter.to_lowercase()))
                .map(|(i, _)| i)
                .collect();
            *selected = filtered.first().copied().unwrap_or(0);
            return;
        }
        if let Some(Overlay::Input { value, .. }) = &mut self.overlay {
            if !ch.is_control() {
                value.push(ch);
            }
            return;
        }
        let search = {
            let Some(state) = self.overlay_list_state_mut() else {
                return;
            };
            state.search.push(ch);
            state.search.clone()
        };
        self.clamp_picker_selection(&search);
    }

    pub fn picker_backspace(&mut self) {
        if let Some(Overlay::ActionMenu {
            filter,
            selected,
            items,
        }) = &mut self.overlay
        {
            filter.pop();
            let filtered: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    filter.is_empty() || item.label.to_lowercase().contains(&filter.to_lowercase())
                })
                .map(|(i, _)| i)
                .collect();
            *selected = filtered.first().copied().unwrap_or(0);
            return;
        }
        if let Some(Overlay::Input { value, .. }) = &mut self.overlay {
            value.pop();
            return;
        }
        let search = {
            let Some(state) = self.overlay_list_state_mut() else {
                return;
            };
            state.search.pop();
            state.search.clone()
        };
        self.clamp_picker_selection(&search);
    }

    fn clamp_picker_selection(&mut self, search: &str) {
        let indices = self.picker_filtered_indices(search);
        let Some(state) = self.overlay_list_state_mut() else {
            return;
        };
        if indices.is_empty() {
            state.selected = 0;
            return;
        }
        if state.selected >= indices.len() {
            state.selected = indices.len() - 1;
        }
    }

    pub async fn picker_confirm(&mut self) {
        let Some(overlay) = self.overlay.clone() else {
            return;
        };
        match overlay {
            Overlay::Context(state) => self.confirm_context_picker(state).await,
            Overlay::Namespace(state) => self.confirm_namespace_picker(state).await,
            Overlay::Container {
                pod_name,
                containers,
                state,
            } => {
                self.confirm_container_picker(pod_name, containers, state)
                    .await
            }
            Overlay::Confirm { action, .. } => {
                self.close_overlay();
                self.run_pending_action(action).await;
            }
            Overlay::Input {
                value,
                purpose,
                extra,
                ..
            } => {
                self.close_overlay();
                self.confirm_input(purpose, value, extra).await;
            }
            Overlay::ActionMenu {
                items, selected, ..
            } => {
                if let Some(item) = items.get(selected) {
                    let action = item.action.clone();
                    self.close_overlay();
                    self.run_pending_action(action).await;
                }
            }
            Overlay::Favorites(state) => self.confirm_favorite_picker(state).await,
            Overlay::PortForwardList { selected } => {
                self.stop_port_forward_at(selected);
            }
            Overlay::Settings { cursor, .. } => {
                self.toggle_settings_item(cursor);
            }
            Overlay::Help => self.close_overlay(),
        }
    }

    async fn confirm_container_picker(
        &mut self,
        pod_name: String,
        containers: Vec<String>,
        state: ListPickerState,
    ) {
        let indices = filter_indices(&containers, &state.search);
        let Some(&container_idx) = indices.get(state.selected) else {
            return;
        };
        let container = containers[container_idx].clone();
        self.close_overlay();
        self.open_log_view(pod_name, Some(container)).await;
    }

    async fn confirm_context_picker(&mut self, state: ListPickerState) {
        let indices = self.filtered_context_indices(&state.search);
        let Some(&context_idx) = indices.get(state.selected) else {
            return;
        };
        let context = self.contexts[context_idx].clone();
        if !context_is_usable(&context) {
            self.error_message = Some(format!(
                "Context \"{context}\" references a missing cluster or user in kubeconfig"
            ));
            self.close_overlay();
            return;
        }

        self.close_overlay();
        self.switch_to_context(&context).await;
    }

    async fn confirm_namespace_picker(&mut self, state: ListPickerState) {
        let indices = self.filtered_namespace_indices(&state.search);
        let Some(&namespace_idx) = indices.get(state.selected) else {
            return;
        };
        let namespace = self.namespaces[namespace_idx].clone();
        self.close_overlay();
        self.switch_to_namespace(&namespace).await;
    }

    async fn switch_to_context(&mut self, context: &str) {
        let kind = self.active_kind;
        let Some(manager) = self.manager.as_mut() else {
            return;
        };

        if let Err(err) = manager.switch_context(context, kind).await {
            self.error_message = Some(err.user_message());
            return;
        }

        self.context_index = self.contexts.iter().position(|c| c == context).unwrap_or(0);
        self.namespaces = manager.list_namespaces().await.unwrap_or_default();
        self.namespace_index = self
            .namespaces
            .iter()
            .position(|n| n == manager.namespace())
            .unwrap_or(0);
        self.crd_targets = manager.crd_targets().to_vec();
        self.selected_crd_index = 0;
        if kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
            manager.set_selected_crd(Some(self.crd_targets[0].clone()));
        }

        self.selected = 0;
        self.clear_detail();
        self.error_message = None;
        if !self.cluster_tabs.iter().any(|t| t == context) {
            self.cluster_tabs.push(context.to_string());
        }
        self.fire_plugins(context);
        self.persist_ui_settings();
        self.refresh().await;
    }

    async fn switch_to_namespace(&mut self, namespace: &str) {
        if let Some(manager) = self.manager.as_mut() {
            if let Err(err) = manager
                .set_namespace(namespace.to_string(), self.active_kind)
                .await
            {
                self.error_message = Some(err.user_message());
                return;
            }
        }
        self.namespace_index = self
            .namespaces
            .iter()
            .position(|n| n == namespace)
            .unwrap_or(0);
        self.selected = 0;
        self.clear_detail();
        self.error_message = None;
        self.refresh().await;
    }

    pub async fn next_crd_target(&mut self) {
        if self.crd_targets.is_empty() {
            return;
        }
        self.selected_crd_index = (self.selected_crd_index + 1) % self.crd_targets.len();
        if let Some(manager) = self.manager.as_mut() {
            let target = self.crd_targets[self.selected_crd_index].clone();
            manager.set_selected_crd(Some(target));
        }
        self.refresh().await;
    }

    pub fn set_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.detail_selection = None;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    fn clear_detail(&mut self) {
        self.detail_yaml.clear();
        self.detail_events.clear();
        self.detail_metrics.clear();
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.detail_selection = None;
        self.clear_detail_search();
    }

    pub fn detail_search_active(&self) -> bool {
        self.detail_search_mode
    }

    pub fn detail_search_visible(&self) -> bool {
        self.detail_search_mode || !self.detail_search_query.is_empty()
    }

    pub fn enter_detail_search(&mut self) {
        self.focus = FocusPane::Detail;
        self.detail_search_mode = true;
        self.detail_selection = None;
    }

    pub fn exit_detail_search(&mut self) {
        self.detail_search_mode = false;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    pub fn clear_detail_search(&mut self) {
        self.detail_search_query.clear();
        self.detail_match_rows.clear();
        self.detail_match_cursor = 0;
        self.detail_search_mode = false;
    }

    pub fn detail_search_push_char(&mut self, ch: char) {
        self.detail_search_query.push(ch);
        self.detail_match_cursor = 0;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    pub fn detail_search_backspace(&mut self) {
        self.detail_search_query.pop();
        self.detail_match_cursor = 0;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    pub fn detail_next_match(&mut self) {
        if self.detail_match_rows.is_empty() {
            return;
        }
        self.detail_match_cursor = (self.detail_match_cursor + 1) % self.detail_match_rows.len();
        self.scroll_detail_to_current_match();
    }

    pub fn detail_prev_match(&mut self) {
        if self.detail_match_rows.is_empty() {
            return;
        }
        self.detail_match_cursor = if self.detail_match_cursor == 0 {
            self.detail_match_rows.len() - 1
        } else {
            self.detail_match_cursor - 1
        };
        self.scroll_detail_to_current_match();
    }

    pub fn detail_current_match_row(&self) -> Option<usize> {
        self.detail_match_rows
            .get(self.detail_match_cursor)
            .copied()
    }

    fn recompute_detail_matches(&mut self) {
        self.detail_match_rows =
            find_detail_match_rows(self.detail_content(), &self.detail_search_query);
        if self.detail_match_cursor >= self.detail_match_rows.len() {
            self.detail_match_cursor = 0;
        }
    }

    fn scroll_detail_to_current_match(&mut self) {
        let Some(&row) = self.detail_match_rows.get(self.detail_match_cursor) else {
            return;
        };
        let visible = self
            .detail_layout
            .map(|l| (l.area.height as usize).max(1))
            .unwrap_or(10);
        let scroll = self.detail_scroll as usize;
        if row < scroll {
            self.detail_scroll = row as u16;
        } else if row >= scroll.saturating_add(visible) {
            self.detail_scroll = row.saturating_sub(visible / 2) as u16;
        }

        if self.detail_search_query.is_empty() {
            return;
        }
        let Some(line) = self.detail_content().lines().nth(row) else {
            return;
        };
        let q = self.detail_search_query.to_lowercase();
        let Some(col) = line.to_lowercase().find(&q) else {
            return;
        };
        // `find` is byte-based on lowercased UTF-8; convert to char index.
        let col = line
            .get(..col)
            .map(|prefix| prefix.chars().count())
            .unwrap_or(0);
        let width = self
            .detail_layout
            .map(|l| (l.area.width as usize).max(1))
            .unwrap_or(40);
        if col < self.detail_scroll_x {
            self.detail_scroll_x = col;
        } else if col >= self.detail_scroll_x.saturating_add(width) {
            self.detail_scroll_x = col.saturating_sub(width / 2);
        }
        self.detail_scroll_x = self.detail_scroll_x.min(self.detail_max_scroll_x(width));
    }

    pub fn detail_content(&self) -> &str {
        match self.detail_tab {
            DetailTab::Describe => &self.detail_yaml,
            DetailTab::Events => &self.detail_events,
            DetailTab::Metrics => &self.detail_metrics,
        }
    }

    pub fn scroll_detail_by(&mut self, delta: i32) {
        self.scroll_detail(delta);
    }

    pub fn detail_pos_at_terminal(&self, row: u16, column: u16) -> Option<(usize, usize)> {
        let layout = self.detail_layout?;
        let area = layout.area;
        if column < area.x
            || column >= area.x.saturating_add(area.width)
            || row < area.y
            || row >= area.y.saturating_add(area.height)
        {
            return None;
        }
        let local_row = (row - area.y) as usize;
        let local_col = (column - area.x) as usize;
        let line = layout.scroll.saturating_add(local_row);
        let content = self.detail_content();
        let line_count = content.lines().count().max(1);
        if line >= line_count && !content.is_empty() {
            return Some((line_count.saturating_sub(1), 0));
        }
        let text = content.lines().nth(line).unwrap_or("");
        let col = local_col
            .saturating_add(layout.scroll_x)
            .min(text.chars().count());
        Some((line, col))
    }

    pub fn detail_contains_pos(&self, row: u16, column: u16) -> bool {
        let Some(layout) = self.detail_layout else {
            return false;
        };
        let area = layout.area;
        column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    pub fn begin_detail_selection(&mut self, line: usize, col: usize) {
        self.focus = FocusPane::Detail;
        self.table_selection = None;
        self.detail_selection = Some(DetailSelection {
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col,
            dragging: true,
        });
    }

    pub fn update_detail_selection(&mut self, line: usize, col: usize) {
        let Some(sel) = self.detail_selection.as_mut() else {
            return;
        };
        if !sel.dragging {
            return;
        }
        sel.end_line = line;
        sel.end_col = col;
    }

    pub fn finish_detail_selection(&mut self) {
        let Some(sel) = self.detail_selection.as_mut() else {
            return;
        };
        sel.dragging = false;
        let ((sl, sc), (el, ec)) = sel.normalized();
        if sl == el && sc == ec {
            self.detail_selection = None;
            return;
        }
        if let Some(text) = self.selected_detail_text() {
            match crate::clipboard::copy_text(&text) {
                Ok(()) => {
                    let preview = if text.len() > 40 {
                        format!("{}…", text.chars().take(40).collect::<String>())
                    } else {
                        text
                    };
                    self.status_message = format!("Copied selection: {preview}");
                    self.error_message = None;
                }
                Err(err) => {
                    self.error_message = Some(format!("Clipboard unavailable: {err}"));
                }
            }
        }
    }

    pub fn selected_detail_text(&self) -> Option<String> {
        let sel = self.detail_selection?;
        let ((sl, sc), (el, ec)) = sel.normalized();
        let lines: Vec<&str> = self.detail_content().lines().collect();
        if lines.is_empty() || sl >= lines.len() {
            return None;
        }
        let el = el.min(lines.len().saturating_sub(1));
        if sl == el {
            let line = lines[sl];
            let start = sc.min(line.chars().count());
            let end = ec.min(line.chars().count()).max(start);
            return Some(line.chars().skip(start).take(end - start).collect());
        }
        let mut out = String::new();
        let first = lines[sl];
        let start = sc.min(first.chars().count());
        out.push_str(&first.chars().skip(start).collect::<String>());
        out.push('\n');
        for line in &lines[sl + 1..el] {
            out.push_str(line);
            out.push('\n');
        }
        let last = lines[el];
        let end = ec.min(last.chars().count());
        out.push_str(&last.chars().take(end).collect::<String>());
        Some(out)
    }

    pub async fn load_detail(&mut self) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let Some(row_idx) = self.selected_row_index() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let Some(row) = self.rows.get(row_idx) else {
            self.error_message = Some("No resource selected".into());
            return;
        };

        self.error_message = None;
        match self.detail_tab {
            DetailTab::Describe => match manager.resource_yaml(self.active_kind, &row.name).await {
                Ok(yaml) => self.detail_yaml = yaml,
                Err(err) => self.detail_yaml = err.user_message(),
            },
            DetailTab::Events => match manager.resource_events(self.active_kind, &row.name).await {
                Ok(events) => self.detail_events = format_events_text(&events),
                Err(err) => self.detail_events = err.user_message(),
            },
            DetailTab::Metrics => {
                if self.active_kind != ResourceKind::Pod {
                    self.detail_metrics =
                        "Metrics are only available for pods (requires metrics-server).".into();
                } else {
                    match manager.pod_metrics().await {
                        Ok(metrics) => {
                            let filtered: Vec<_> = metrics
                                .into_iter()
                                .filter(|m| m.pod_name == row.name)
                                .collect();
                            if filtered.is_empty() {
                                self.detail_metrics =
                                    "No metrics for this pod (is metrics-server installed?)".into();
                            } else {
                                self.detail_metrics = format_metrics_text(&filtered);
                            }
                        }
                        Err(err) => self.detail_metrics = err.user_message(),
                    }
                }
            }
        }
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    pub async fn start_logs_for_selection(&mut self) {
        if self.active_kind != ResourceKind::Pod {
            self.error_message = Some("Logs are only available for pods.".into());
            return;
        }
        let Some(row_idx) = self.selected_row_index() else {
            self.error_message = Some("No pod selected.".into());
            return;
        };
        let Some(row) = self.rows.get(row_idx) else {
            self.error_message = Some("No pod selected.".into());
            return;
        };
        let pod_name = row.name.clone();
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        match manager.pod_containers(&pod_name).await {
            Ok(containers) if containers.len() > 1 => {
                self.overlay = Some(Overlay::Container {
                    pod_name,
                    containers: containers.into_iter().map(|c| c.name).collect(),
                    state: ListPickerState {
                        search: String::new(),
                        selected: 0,
                    },
                });
            }
            Ok(containers) => {
                let container = containers.first().map(|c| c.name.clone());
                self.open_log_view(pod_name, container).await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    pub async fn start_logs_for_service_selection(&mut self) {
        if self.active_kind != ResourceKind::Service {
            self.error_message = Some("Service logs are only available for Services.".into());
            return;
        }
        let Some(row_idx) = self.selected_row_index() else {
            self.error_message = Some("No service selected.".into());
            return;
        };
        let Some(row) = self.rows.get(row_idx) else {
            self.error_message = Some("No service selected.".into());
            return;
        };
        let service_name = row.name.clone();
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        match tokio::time::timeout(
            std::time::Duration::from_secs(20),
            manager.pods_for_service(&service_name),
        )
        .await
        {
            Ok(Ok(pods)) if pods.is_empty() => {
                self.error_message = Some(format!(
                    "No pods match service `{service_name}` (missing or empty selector?)."
                ));
            }
            Ok(Ok(mut pods)) => {
                const MAX_SERVICE_LOG_PODS: usize = 40;
                if pods.len() > MAX_SERVICE_LOG_PODS {
                    let dropped = pods.len() - MAX_SERVICE_LOG_PODS;
                    pods.truncate(MAX_SERVICE_LOG_PODS);
                    self.status_message = format!(
                        "Service has many pods; streaming first {MAX_SERVICE_LOG_PODS} (+{dropped} skipped)."
                    );
                }
                let title = format!("svc/{service_name} ({} pods)", pods.len());
                self.open_multi_pod_log_view(title, pods).await;
            }
            Ok(Err(err)) => self.error_message = Some(err.user_message()),
            Err(_) => {
                self.error_message = Some(format!(
                    "Timed out listing pods for service `{service_name}`."
                ));
            }
        }
    }

    async fn open_log_view(&mut self, pod_name: String, container: Option<String>) {
        self.close_log_view();
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        let (line_tx, line_rx) = mpsc::channel(8_192);
        let (err_tx, err_rx) = mpsc::channel(8);
        let stream_task =
            manager.spawn_log_stream(pod_name.clone(), container.clone(), true, line_tx, err_tx);

        self.log_view = Some(LogView {
            pod_name,
            container,
            lines: Vec::new(),
            wrapped_cache: Vec::new(),
            wrapped_sources: Vec::new(),
            wrap_cache_width: 0,
            scroll: 0,
            follow: true,
            visible_lines: 1,
            wrap_width: 80,
            search_mode: false,
            search_query: String::new(),
            match_rows: Vec::new(),
            match_cursor: 0,
            matches_dirty: false,
            error: None,
            highlight_source: None,
            line_rx,
            err_rx,
            stream_task,
        });
        self.error_message = None;
    }

    async fn open_multi_pod_log_view(&mut self, title: String, pod_names: Vec<String>) {
        self.close_log_view();
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        let (line_tx, line_rx) = mpsc::channel(8_192);
        let (err_tx, err_rx) = mpsc::channel(8);
        let stream_task = manager.spawn_multi_pod_log_stream(pod_names, true, line_tx, err_tx);

        self.log_view = Some(LogView {
            pod_name: title,
            container: None,
            lines: Vec::new(),
            wrapped_cache: Vec::new(),
            wrapped_sources: Vec::new(),
            wrap_cache_width: 0,
            scroll: 0,
            follow: true,
            visible_lines: 1,
            wrap_width: 80,
            search_mode: false,
            search_query: String::new(),
            match_rows: Vec::new(),
            match_cursor: 0,
            matches_dirty: false,
            error: None,
            highlight_source: None,
            line_rx,
            err_rx,
            stream_task,
        });
        self.error_message = None;
        self.status_message = "Polling service pod logs — Esc/q to close".into();
    }

    pub fn log_scroll(&mut self, delta: i32) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.follow = false;
        let max = log.max_scroll();
        let next = log.scroll as i32 + delta;
        log.scroll = next.clamp(0, max as i32) as usize;
    }

    pub fn log_follow_bottom(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.follow = true;
        log.scroll_to_bottom();
    }

    pub fn log_toggle_follow(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.follow = !log.follow;
        if log.follow {
            log.scroll_to_bottom();
        }
    }

    pub fn log_scroll_top(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.follow = false;
        log.scroll = 0;
    }

    pub fn prepare_log_view(&mut self, visible_lines: usize, wrap_width: usize) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.visible_lines = visible_lines.max(1);
        let width = wrap_width.max(1);
        if log.wrap_width != width {
            log.wrap_width = width;
            log.invalidate_wrap_cache();
        }
        log.ensure_wrap_cache();
        if log.matches_dirty {
            log.recompute_matches();
        }
        if log.follow {
            log.scroll_to_bottom();
        } else {
            log.scroll = log.scroll.min(log.max_scroll());
        }
    }

    pub fn log_search_active(&self) -> bool {
        self.log_view.as_ref().is_some_and(|log| log.search_mode)
    }

    pub fn enter_log_search(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_mode = true;
        log.follow = false;
    }

    pub fn clear_log_search(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.clear();
        log.match_rows.clear();
        log.match_cursor = 0;
        log.search_mode = false;
        log.matches_dirty = false;
    }

    pub fn exit_log_search(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_mode = false;
        if log.matches_dirty {
            log.recompute_matches();
        }
        if !log.search_query.is_empty() {
            log.scroll_to_current_match();
        }
    }

    pub fn log_search_push_char(&mut self, ch: char) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.push(ch);
        log.matches_dirty = true;
        log.recompute_matches();
        log.scroll_to_current_match();
    }

    pub fn log_search_backspace(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.pop();
        log.matches_dirty = true;
        log.recompute_matches();
        log.scroll_to_current_match();
    }

    pub fn log_next_match(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        if log.match_rows.is_empty() {
            return;
        }
        log.match_cursor = (log.match_cursor + 1) % log.match_rows.len();
        log.scroll_to_current_match();
    }

    pub fn log_prev_match(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        if log.match_rows.is_empty() {
            return;
        }
        log.match_cursor = if log.match_cursor == 0 {
            log.match_rows.len() - 1
        } else {
            log.match_cursor - 1
        };
        log.scroll_to_current_match();
    }

    pub fn yank_log_line_at_wrapped_row(&mut self, wrapped_row: usize) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        let source = log.source_line_for_wrapped_row(wrapped_row);
        let Some(text) = log.lines.get(source).cloned() else {
            return;
        };
        match crate::clipboard::copy_text(&text) {
            Ok(()) => {
                log.highlight_source = Some(source);
                self.status_message = format!(
                    "Copied full log line to clipboard ({}/{})",
                    source + 1,
                    log.lines.len()
                );
                self.error_message = None;
            }
            Err(err) => {
                self.error_message = Some(format!("Clipboard unavailable: {err}"));
            }
        }
    }

    pub fn wrapped_row_at_terminal_pos(&self, row: u16, column: u16) -> Option<usize> {
        let layout = self.log_layout?;
        let area = layout.area;
        if column < area.x
            || column >= area.x.saturating_add(area.width)
            || row < area.y
            || row >= area.y.saturating_add(area.height)
        {
            return None;
        }
        let local = (row - area.y) as usize;
        let wrapped_row = layout.scroll.saturating_add(local);
        let log = self.log_view.as_ref()?;
        if wrapped_row < log.wrapped_lines().len() {
            Some(wrapped_row)
        } else {
            None
        }
    }
}

impl LogView {
    pub fn wrapped_lines(&self) -> &[String] {
        &self.wrapped_cache
    }

    pub fn source_line_for_wrapped_row(&self, wrapped_row: usize) -> usize {
        self.wrapped_sources
            .get(wrapped_row)
            .copied()
            .unwrap_or_else(|| self.lines.len().saturating_sub(1))
    }

    fn invalidate_wrap_cache(&mut self) {
        self.wrap_cache_width = 0;
        self.wrapped_cache.clear();
        self.wrapped_sources.clear();
        if !self.search_query.is_empty() {
            self.matches_dirty = true;
        }
    }

    fn ensure_wrap_cache(&mut self) {
        let width = self.wrap_width.max(1);
        if self.wrap_cache_width == width
            && self.wrapped_sources.len() == self.wrapped_cache.len()
            && cache_covers_lines(&self.wrapped_sources, self.lines.len())
        {
            return;
        }
        self.rebuild_wrap_cache();
    }

    fn rebuild_wrap_cache(&mut self) {
        let width = self.wrap_width.max(1);
        let (wrapped, sources) = wrap_log_lines_with_sources(&self.lines, width);
        self.wrapped_cache = wrapped;
        self.wrapped_sources = sources;
        self.wrap_cache_width = width;
        if !self.search_query.is_empty() {
            self.matches_dirty = true;
        }
    }

    fn max_scroll(&self) -> usize {
        self.wrapped_cache
            .len()
            .saturating_sub(self.visible_lines.max(1))
    }

    fn scroll_to_bottom(&mut self) {
        self.ensure_wrap_cache();
        self.scroll = self.max_scroll();
    }

    fn recompute_matches(&mut self) {
        self.ensure_wrap_cache();
        self.match_rows = find_log_match_rows(&self.wrapped_cache, &self.search_query);
        if self.match_cursor >= self.match_rows.len() {
            self.match_cursor = 0;
        }
        self.matches_dirty = false;
    }

    fn scroll_to_current_match(&mut self) {
        let Some(&row) = self.match_rows.get(self.match_cursor) else {
            return;
        };
        self.follow = false;
        self.ensure_wrap_cache();
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll.saturating_add(self.visible_lines) {
            self.scroll = row.saturating_sub(self.visible_lines / 2);
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    pub fn current_match_row(&self) -> Option<usize> {
        self.match_rows.get(self.match_cursor).copied()
    }

    /// Append one source line; wrap segments are added incrementally when cache is warm.
    fn push_line(&mut self, line: String) {
        let source_idx = self.lines.len();
        let width = self.wrap_width.max(1);
        if self.wrap_cache_width == width {
            append_wrapped_line(
                &mut self.wrapped_cache,
                &mut self.wrapped_sources,
                &line,
                source_idx,
                width,
            );
        } else {
            self.invalidate_wrap_cache();
        }
        self.lines.push(line);
        if !self.search_query.is_empty() {
            self.matches_dirty = true;
        }
        if self.lines.len() > LOG_BUFFER_MAX_LINES {
            let excess = self.lines.len() - LOG_BUFFER_MAX_LINES;
            self.lines.drain(0..excess);
            self.invalidate_wrap_cache();
            if let Some(src) = self.highlight_source.as_mut() {
                *src = src.saturating_sub(excess);
            }
        }
    }

    fn after_batch_ingest(&mut self) {
        self.ensure_wrap_cache();
        if self.matches_dirty {
            // Defer full rematch until paint if the query is active; cheap path when empty.
            if self.search_query.is_empty() {
                self.matches_dirty = false;
                self.match_rows.clear();
            }
        }
        if self.follow {
            self.scroll = self.max_scroll();
        } else {
            self.scroll = self.scroll.min(self.max_scroll());
        }
    }
}

fn cache_covers_lines(sources: &[usize], line_count: usize) -> bool {
    match sources.last() {
        None => line_count == 0,
        Some(&last) => last + 1 == line_count,
    }
}

fn append_wrapped_line(
    wrapped: &mut Vec<String>,
    sources: &mut Vec<usize>,
    line: &str,
    source_idx: usize,
    width: usize,
) {
    let width = width.max(1);
    if line.is_empty() {
        wrapped.push(String::new());
        sources.push(source_idx);
        return;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + width).min(chars.len());
        wrapped.push(chars[start..end].iter().collect());
        sources.push(source_idx);
        start = end;
    }
}

fn wrap_log_lines_with_sources(lines: &[String], width: usize) -> (Vec<String>, Vec<usize>) {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    let mut sources = Vec::new();
    for (source_idx, line) in lines.iter().enumerate() {
        append_wrapped_line(&mut wrapped, &mut sources, line, source_idx, width);
    }
    (wrapped, sources)
}

fn find_detail_match_rows(content: &str, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

fn find_log_match_rows(wrapped: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let q = query.to_lowercase();
    wrapped
        .iter()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&q))
        .map(|(idx, _)| idx)
        .collect()
}

fn filter_indices(items: &[String], search: &str) -> Vec<usize> {
    let query = search.to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
        .map(|(idx, _)| idx)
        .collect()
}
