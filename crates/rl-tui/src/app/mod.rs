mod features;

use std::collections::HashMap;
use std::process::Child;
use std::sync::Arc;

use ratatui::layout::Rect;

use rl_core::{
    config::context_is_usable, format_events_text, format_metrics_text, load_cluster_cache,
    load_settings, save_cluster_cache, save_settings, ClusterCache, ClusterDashboard,
    ClusterManager, ContainerInfo, CrdTarget, FavoriteResource, PortForwardHandle, ResourceKind,
    ResourceRow, LOG_BUFFER_MAX_LINES,
};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;

pub type SharedManager = Arc<RwLock<ClusterManager>>;

/// One candidate editor offered in the first-run picker.
#[derive(Debug, Clone)]
pub struct EditorCandidate {
    pub label: &'static str,
    /// Binary name looked up on `PATH`.
    pub binary: &'static str,
    /// Value persisted to `settings.json` (includes `--wait` for GUI editors).
    pub command: &'static str,
}

/// Editors offered when `settings.editor` is unset. GUI editors get `--wait` so the
/// TUI reads back the file only after the editor closes.
pub fn editor_candidates() -> &'static [EditorCandidate] {
    static CANDIDATES: &[EditorCandidate] = &[
        EditorCandidate {
            label: "Zed",
            binary: "zed",
            command: "zed --wait",
        },
        EditorCandidate {
            label: "VS Code",
            binary: "code",
            command: "code --wait",
        },
        EditorCandidate {
            label: "Neovim",
            binary: "nvim",
            command: "nvim",
        },
        EditorCandidate {
            label: "Vim",
            binary: "vim",
            command: "vim",
        },
        EditorCandidate {
            label: "Helix",
            binary: "hx",
            command: "hx",
        },
        EditorCandidate {
            label: "micro",
            binary: "micro",
            command: "micro",
        },
        EditorCandidate {
            label: "Emacs",
            binary: "emacs",
            command: "emacs",
        },
        EditorCandidate {
            label: "nano",
            binary: "nano",
            command: "nano",
        },
        EditorCandidate {
            label: "Sublime Text",
            binary: "subl",
            command: "subl --wait",
        },
    ];
    CANDIDATES
}

/// True if `binary` exists on `PATH`. No process spawn — scans `PATH` dirs.
pub fn editor_installed(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        candidate.is_file()
    })
}

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
    SetEditor,
    TriggerCronJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsCursor {
    NativePortForward,
    ExternalLogs,
    Theme,
    AddKubeconfigPath,
    ExtraKubeconfigList,
    Editor,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerPickerPurpose {
    Logs,
    ExternalLogs,
    Exec,
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
        purpose: ContainerPickerPurpose,
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
        /// Secondary value (remote port for two-step PF, cronjob name for trigger).
        extra: Option<String>,
        /// Edit cursor as a char index into `value` (0..=char count).
        cursor: usize,
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
    /// First-run editor picker shown when `settings.editor` is unset.
    EditorPicker(ListPickerState),
    Settings {
        cursor: SettingsCursor,
        path_selected: usize,
    },
    Help,
}

impl Overlay {
    /// Input overlay with the edit cursor placed at the end of the initial value.
    pub fn input(
        prompt: String,
        value: String,
        purpose: InputPurpose,
        extra: Option<String>,
    ) -> Self {
        let cursor = value.chars().count();
        Overlay::Input {
            prompt,
            value,
            purpose,
            extra,
            cursor,
        }
    }
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
    /// Last clicked / focused wrapped row (for `y` / Ctrl+C).
    pub focus_wrapped_row: Option<usize>,
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

/// Screen region of the detail body — mouse scroll hit-testing.
#[derive(Debug, Clone, Copy)]
pub struct DetailLayout {
    /// Full detail column (tabs + body) for coarse mouse hit-testing.
    pub panel: Rect,
    /// Text viewport only.
    pub area: Rect,
    /// Bottom horizontal scrollbar track, if shown.
    pub hscroll_area: Option<Rect>,
    /// Visible text columns (excludes vertical scrollbar gutter).
    pub text_width: usize,
    #[allow(dead_code)]
    pub scroll: usize,
    #[allow(dead_code)]
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
    pub manager: Option<SharedManager>,
    /// Mirrored for sync UI draws (updated on connect / context / namespace switch).
    pub active_context: String,
    pub active_namespace: String,
    pub active_kind: ResourceKind,
    pub sidebar_index: usize,
    pub focus: FocusPane,
    pub view_mode: ViewMode,
    pub rows: Vec<ResourceRow>,
    pub selected: usize,
    pub contexts: Vec<String>,
    pub context_index: usize,
    /// Namespaces for the active context (picker + status bar).
    pub namespaces: Vec<String>,
    pub namespace_index: usize,
    pub namespaces_by_context: HashMap<String, Vec<String>>,
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
    /// Full-frame cell grid captured after draw — enables selecting any on-screen text.
    pub screen_cells: Vec<Vec<String>>,
    /// Drag selection in terminal coordinates (`line` = row, `col` = column).
    pub screen_selection: Option<TextSelection>,
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
    pub external_logs: bool,
    pub extra_kubeconfig_paths: Vec<String>,
    pub editor: Option<String>,
    /// Detected editors for the first-run picker. Order matches `editor_candidates()`.
    pub editor_candidates: Vec<EditorCandidate>,
    /// In-memory cache of non-watch kind rows (Helm, CRD instances) so switching
    /// back to a previously loaded kind renders instantly while a refresh runs.
    /// Key: (kind, namespace, selected CRD display name).
    pub kind_row_cache: HashMap<(ResourceKind, String, Option<String>), Vec<ResourceRow>>,
    /// Background kind-row load in flight (Helm/CRD). Polled each frame.
    pub kind_load: Option<KindLoad>,
    pub port_forwards: HashMap<u64, PortForwardSession>,
    pub port_forward_entries: Vec<PortForwardEntry>,
    pub next_port_forward_id: u64,
    pub dashboard: Option<ClusterDashboard>,
    /// Set by action handlers so main loop can suspend TUI for editor/exec.
    pub pending_external: Option<ExternalRequest>,
    connect_attempted: bool,
    /// Non-blocking connect so crossterm keeps draining mouse/key input.
    connect_task: Option<tokio::task::JoinHandle<Result<ClusterManager, rl_core::Error>>>,
    /// Non-blocking slow ops (context switch, namespace switch, describe) so the
    /// input loop stays responsive. Tasks clone `SharedManager` and lock briefly.
    pending_op: Option<PendingOp>,
    /// Pending log-open request set by a finished FetchContainers/FetchServicePods
    /// op — main loop opens the view next tick (needs async + &manager).
    pending_open_log: Option<PendingLogOpen>,
}

#[derive(Clone)]
pub enum PendingLogOpen {
    Pod {
        pod_name: String,
        container: Option<String>,
    },
    MultiPod {
        title: String,
        pod_names: Vec<String>,
    },
}

/// In-flight background op spawned off the crossterm input loop.
pub enum PendingOp {
    SwitchContext {
        context: String,
        handle: tokio::task::JoinHandle<SwitchContextOutcome>,
    },
    SwitchNamespace {
        namespace: String,
        handle: tokio::task::JoinHandle<SwitchNamespaceOutcome>,
    },
    LoadDetail {
        handle: tokio::task::JoinHandle<LoadDetailOutcome>,
    },
    FetchContainers {
        pod_name: String,
        external: bool,
        handle: tokio::task::JoinHandle<FetchContainersOutcome>,
    },
    FetchServicePods {
        service_name: String,
        external: bool,
        handle: tokio::task::JoinHandle<FetchServicePodsOutcome>,
    },
    Refresh {
        handle: tokio::task::JoinHandle<RefreshOutcome>,
    },
}

/// Background load of non-watch kind rows (Helm releases, CRD instances).
/// Separate from `PendingOp` so multiple kind switches don't clobber exclusive ops
/// and so cached rows render instantly while the refresh runs.
pub struct KindLoad {
    pub kind: ResourceKind,
    /// Cache key suffix: selected CRD target's display name for CRD, else None.
    pub crd_name: Option<String>,
    pub handle: tokio::task::JoinHandle<Result<Vec<ResourceRow>, rl_core::Error>>,
}

pub type SwitchContextOutcome = Result<SwitchContextResult, rl_core::Error>;

#[derive(Clone)]
pub struct SwitchContextResult {
    pub namespaces: Vec<String>,
    pub namespace_index: usize,
    pub crd_targets: Vec<CrdTarget>,
}

pub type SwitchNamespaceOutcome = Result<SwitchNamespaceResult, rl_core::Error>;

#[derive(Clone, Default)]
pub struct SwitchNamespaceResult {
    pub rows: Vec<ResourceRow>,
}

pub type LoadDetailOutcome = Result<LoadDetailResult, rl_core::Error>;

#[derive(Clone)]
pub enum LoadDetailResult {
    Describe(String),
    Events(String),
    Metrics(String),
}

pub type FetchContainersOutcome = Result<Vec<ContainerInfo>, rl_core::Error>;

pub type RefreshOutcome = Result<RefreshResult, rl_core::Error>;

#[derive(Clone)]
pub struct RefreshResult {
    pub contexts: Vec<String>,
    pub namespaces: Vec<String>,
    pub namespace_index: usize,
    pub rows: Vec<ResourceRow>,
}

pub type FetchServicePodsOutcome = Result<rl_core::ServicePods, rl_core::Error>;

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
    pub fn new() -> Self {
        let settings = load_settings();
        let disk_cache = load_cluster_cache();
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
            active_context: String::new(),
            active_namespace: String::new(),
            active_kind: ResourceKind::Pod,
            sidebar_index: 0,
            focus: FocusPane::Table,
            view_mode: ViewMode::Browser,
            rows: Vec::new(),
            selected: 0,
            contexts: disk_cache.contexts,
            context_index: 0,
            namespaces: Vec::new(),
            namespace_index: 0,
            namespaces_by_context: disk_cache.namespaces_by_context,
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
            screen_cells: Vec::new(),
            screen_selection: None,
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
            external_logs: settings.external_logs,
            extra_kubeconfig_paths: settings.extra_kubeconfig_paths.clone(),
            editor: settings.editor.clone(),
            editor_candidates: editor_candidates().to_vec(),
            kind_row_cache: HashMap::new(),
            kind_load: None,
            port_forwards: HashMap::new(),
            port_forward_entries: Vec::new(),
            next_port_forward_id: 1,
            dashboard: None,
            pending_external: None,
            connect_attempted: false,
            connect_task: None,
            pending_op: None,
            pending_open_log: None,
        }
    }

    fn cluster_cache_snapshot(&self) -> ClusterCache {
        ClusterCache {
            contexts: self.contexts.clone(),
            namespaces_by_context: self.namespaces_by_context.clone(),
        }
    }

    fn save_cluster_cache_async(&self) {
        let cache = self.cluster_cache_snapshot();
        tokio::task::spawn_blocking(move || {
            let _ = save_cluster_cache(&cache);
        });
    }

    fn sync_namespaces_for_context(&mut self, context: &str) {
        if let Some(cached) = self.namespaces_by_context.get(context).cloned() {
            self.namespaces = cached;
            self.namespace_index = self
                .namespaces
                .iter()
                .position(|n| n == self.active_namespace.as_str())
                .unwrap_or(0);
        }
    }

    fn update_namespace_cache(&mut self, context: &str, namespaces: Vec<String>) {
        self.namespaces_by_context
            .insert(context.to_string(), namespaces.clone());
        if context == self.active_context {
            self.namespaces = namespaces;
        }
        self.save_cluster_cache_async();
    }

    /// True while a heavy exclusive op is in flight (context / namespace / full refresh).
    pub fn exclusive_op_open(&self) -> bool {
        matches!(
            self.pending_op,
            Some(
                PendingOp::SwitchContext { .. }
                    | PendingOp::SwitchNamespace { .. }
                    | PendingOp::Refresh { .. }
            )
        )
    }

    fn abort_lightweight_pending(&mut self) {
        let Some(op) = self.pending_op.take() else {
            return;
        };
        match op {
            PendingOp::LoadDetail { handle } => handle.abort(),
            PendingOp::FetchContainers { handle, .. } => handle.abort(),
            PendingOp::FetchServicePods { handle, .. } => handle.abort(),
            other => self.pending_op = Some(other),
        }
    }

    pub fn persist_ui_settings(&self) {
        let mut settings = load_settings();
        settings.theme = Some(self.theme_mode.as_str().to_string());
        settings.open_cluster_tabs = self.cluster_tabs.clone();
        settings.favorites = self.favorites.clone();
        settings.use_native_port_forward = self.use_native_port_forward;
        settings.external_logs = self.external_logs;
        settings.extra_kubeconfig_paths = self.extra_kubeconfig_paths.clone();
        settings.editor = self.editor.clone();
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

    /// External `kubectl logs -f` exits immediately for terminated pods
    /// (e.g. finished CronJob runs), closing the terminal window before the
    /// logs can be read — those open in the built-in view instead.
    pub fn logs_external_for(&self, pod_status: &str) -> bool {
        self.external_logs && !matches!(pod_status, "Succeeded" | "Failed")
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

    /// True when the open overlay takes typed input (picker filter / prompt value),
    /// so plain letters must reach the field instead of being used as shortcuts.
    pub fn overlay_accepts_text(&self) -> bool {
        matches!(
            self.overlay,
            Some(
                Overlay::Context(_)
                    | Overlay::Namespace(_)
                    | Overlay::Container { .. }
                    | Overlay::Favorites(_)
                    | Overlay::ActionMenu { .. }
                    | Overlay::Input { .. }
            )
        )
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

    /// Start a background connect on first launch (does not block the event loop).
    pub fn kickoff_connect_if_needed(&mut self) {
        if self.connect_task.is_some()
            || matches!(
                self.connection,
                ConnectionState::Connecting | ConnectionState::Connected
            )
            || self.connect_attempted
        {
            return;
        }
        self.spawn_connect();
    }

    fn spawn_connect(&mut self) {
        if self.connect_task.is_some() {
            return;
        }
        self.connection = ConnectionState::Connecting;
        self.connect_attempted = true;
        self.error_message = None;
        self.status_message = "Connecting...".into();
        let kind = self.active_kind;
        self.connect_task = Some(tokio::spawn(async move {
            ClusterManager::connect_default(kind).await
        }));
    }

    /// Abort an in-flight connect so quit/`q` is never stuck waiting on kube auth.
    pub fn cancel_connect(&mut self) {
        if let Some(handle) = self.connect_task.take() {
            handle.abort();
        }
        if matches!(self.connection, ConnectionState::Connecting) {
            self.connection = ConnectionState::Disconnected;
            self.status_message = "Connect cancelled.".into();
        }
    }

    /// Apply a finished background connect without stalling crossterm reads.
    pub async fn poll_connect(&mut self) {
        let Some(handle) = self.connect_task.as_ref() else {
            return;
        };
        if !handle.is_finished() {
            return;
        }
        let Some(handle) = self.connect_task.take() else {
            return;
        };
        match handle.await {
            Ok(Ok(manager)) => self.apply_connected_manager(manager).await,
            Ok(Err(err)) => {
                let msg = err.user_message();
                self.connection = ConnectionState::Failed(msg.clone());
                self.manager = None;
                self.status_message.clear();
                self.error_message = Some(msg);
            }
            Err(err) if err.is_cancelled() => {
                self.connection = ConnectionState::Disconnected;
                self.status_message = "Connect cancelled.".into();
            }
            Err(err) => {
                let msg = format!("Connect task failed: {err}");
                self.connection = ConnectionState::Failed(msg.clone());
                self.manager = None;
                self.status_message.clear();
                self.error_message = Some(msg);
            }
        }
    }

    async fn apply_connected_manager(&mut self, manager: ClusterManager) {
        // Mark connected before slow follow-up so the UI/input loop stays responsive.
        self.connection = ConnectionState::Connected;
        self.status_message = "Connected".into();
        self.error_message = None;

        let shared = Arc::new(RwLock::new(manager));
        let mut guard = shared.write().await;

        let ctx = guard.context().to_string();
        let active_ns = guard.namespace().to_string();
        self.active_context = ctx.clone();
        self.active_namespace = active_ns.clone();

        let namespaces = if let Some(cached) = self.namespaces_by_context.get(&ctx) {
            cached.clone()
        } else {
            let listed = guard.list_namespaces().await.unwrap_or_default();
            self.update_namespace_cache(&ctx, listed.clone());
            listed
        };
        let namespace_index = namespaces
            .iter()
            .position(|n| n == active_ns.as_str())
            .unwrap_or(0);
        self.crd_targets = guard.crd_targets().to_vec();
        if self.active_kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
            let target = self.crd_targets[self
                .selected_crd_index
                .min(self.crd_targets.len().saturating_sub(1))]
            .clone();
            guard.set_selected_crd(Some(target));
        }
        self.namespaces = namespaces;
        self.namespace_index = namespace_index;
        if !self.cluster_tabs.iter().any(|t| t == &ctx) {
            self.cluster_tabs.push(ctx.clone());
        }
        drop(guard);
        self.manager = Some(shared);
        self.sync_context_list().await;
        self.fire_plugins(&ctx);
        self.persist_ui_settings();
        self.pull_rows_now().await;
        self.update_status_from_rows();
    }

    pub async fn retry_connect(&mut self) {
        if self.connect_task.is_some() {
            return;
        }
        self.connection = ConnectionState::Disconnected;
        self.connect_attempted = false;
        self.spawn_connect();
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, ConnectionState::Connected)
    }

    /// Soft-refuse starting another exclusive op while one is in flight.
    pub(super) fn busy_soft_refuse(&mut self) -> bool {
        if self.exclusive_op_open() {
            if self.status_message.is_empty() || !self.status_message.starts_with("Busy") {
                self.status_message = "Busy — wait for current request (or press q to quit)".into();
            }
            true
        } else {
            false
        }
    }

    fn update_status_from_rows(&mut self) {
        if self.active_context.is_empty() {
            return;
        }
        self.status_message = format!(
            "{} / {} — {} items",
            self.active_context,
            self.active_namespace,
            self.rows.len()
        );
    }

    /// Snapshot watched kinds without blocking on a write lock or restarting watches.
    pub async fn pull_rows_now(&mut self) {
        let Some(manager) = self.manager.clone() else {
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
            let mut guard = manager.write().await;
            guard.set_selected_crd(Some(target));
            drop(guard);
        }
        if kind.uses_watch() {
            if let Ok(guard) = manager.try_read() {
                self.rows = guard.snapshot(kind).rows;
            }
            if self.selected >= self.rows.len() {
                self.selected = self.rows.len().saturating_sub(1);
            }
            self.clamp_table_selection();
            self.update_status_from_rows();
        } else {
            // Non-watch kinds (Helm, CRD): don't block the input loop. Render cached
            // rows instantly if present, then refresh in the background.
            self.load_kind_rows_async().await;
        }
    }

    /// Cache key for non-watch kind rows: (kind, namespace, CRD display name?).
    fn kind_cache_key(&self, kind: ResourceKind) -> (ResourceKind, String, Option<String>) {
        let crd_name = if kind == ResourceKind::Crd {
            self.crd_targets
                .get(
                    self.selected_crd_index
                        .min(self.crd_targets.len().saturating_sub(1)),
                )
                .map(|t| t.display_name.clone())
        } else {
            None
        };
        (kind, self.active_namespace.clone(), crd_name)
    }

    /// Non-blocking load for non-watch kinds (Helm releases, CRD instances). Renders
    /// cached rows instantly if available, then spawns a background `list_rows` and
    /// fills in fresh rows when it lands. Watched kinds stay on the snapshot path.
    pub async fn load_kind_rows_async(&mut self) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let kind = self.active_kind;
        if kind.uses_watch() {
            return;
        }
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
            let mut guard = manager.write().await;
            guard.set_selected_crd(Some(target));
            drop(guard);
        }
        let key = self.kind_cache_key(kind);
        let cached = self.kind_row_cache.get(&key).cloned();
        match &cached {
            Some(rows) => {
                self.rows = rows.clone();
                if self.selected >= self.rows.len() {
                    self.selected = self.rows.len().saturating_sub(1);
                }
                self.clamp_table_selection();
            }
            None => {
                // No cache yet: clear stale rows from the previous kind so we don't
                // render Pods under a "CRD instances" header while loading.
                self.rows.clear();
                self.selected = 0;
            }
        }
        let label = kind.label();
        self.status_message = if cached.is_some() {
            format!("{label} — refreshing…")
        } else {
            format!("Loading {label}…")
        };
        // Abort any prior in-flight kind load so we don't apply stale results.
        if let Some(prev) = self.kind_load.take() {
            prev.handle.abort();
        }
        let work_kind = kind;
        let handle = tokio::spawn(async move {
            let guard = manager.read().await;
            guard.list_rows(work_kind).await
        });
        self.kind_load = Some(KindLoad {
            kind,
            crd_name: key.2.clone(),
            handle,
        });
        self.update_status_from_rows();
    }

    /// Poll the in-flight non-watch kind load. On completion: update the cache and,
    /// if still viewing that kind, render fresh rows.
    pub async fn poll_kind_load(&mut self) {
        let finished = self
            .kind_load
            .as_ref()
            .map(|l| l.handle.is_finished())
            .unwrap_or(false);
        if !finished {
            return;
        }
        let Some(load) = self.kind_load.take() else {
            return;
        };
        let KindLoad {
            kind,
            crd_name,
            handle,
        } = load;
        match handle.await {
            Ok(Ok(rows)) => {
                let key = (kind, self.active_namespace.clone(), crd_name.clone());
                self.kind_row_cache.insert(key, rows.clone());
                if self.active_kind == kind {
                    self.rows = rows;
                    if self.selected >= self.rows.len() {
                        self.selected = self.rows.len().saturating_sub(1);
                    }
                    self.clamp_table_selection();
                    self.status_message.clear();
                    self.error_message = None;
                    self.update_status_from_rows();
                }
            }
            Ok(Err(err)) => {
                if self.active_kind == kind {
                    self.error_message = Some(err.user_message());
                    self.status_message.clear();
                }
            }
            Err(err) => {
                if self.active_kind == kind {
                    self.error_message = Some(format!("Kind load task failed: {err}"));
                    self.status_message.clear();
                }
            }
        }
    }

    /// Non-blocking poll: if the in-flight op finished, apply its result.
    pub async fn poll_pending_op(&mut self) {
        let Some(op) = self.pending_op.as_ref() else {
            return;
        };
        let finished = match op {
            PendingOp::SwitchContext { handle, .. } => handle.is_finished(),
            PendingOp::SwitchNamespace { handle, .. } => handle.is_finished(),
            PendingOp::LoadDetail { handle, .. } => handle.is_finished(),
            PendingOp::FetchContainers { handle, .. } => handle.is_finished(),
            PendingOp::FetchServicePods { handle, .. } => handle.is_finished(),
            PendingOp::Refresh { handle } => handle.is_finished(),
        };
        if !finished {
            return;
        }
        let Some(op) = self.pending_op.take() else {
            return;
        };
        match op {
            PendingOp::SwitchContext { context, handle } => match handle.await {
                Ok(outcome) => {
                    self.apply_switch_context(&context, outcome).await;
                }
                Err(err) => {
                    self.error_message = Some(format!("Context switch task failed: {err}"));
                    self.status_message.clear();
                }
            },
            PendingOp::SwitchNamespace { namespace, handle } => match handle.await {
                Ok(outcome) => {
                    self.apply_switch_namespace(&namespace, outcome);
                }
                Err(err) => {
                    self.error_message = Some(format!("Namespace switch task failed: {err}"));
                    self.status_message.clear();
                }
            },
            PendingOp::LoadDetail { handle, .. } => match handle.await {
                Ok(outcome) => {
                    self.apply_load_detail(outcome);
                }
                Err(err) => {
                    self.error_message = Some(format!("Describe task failed: {err}"));
                    self.status_message.clear();
                }
            },
            PendingOp::FetchContainers {
                pod_name,
                external,
                handle,
            } => match handle.await {
                Ok(outcome) => {
                    self.apply_fetch_containers(pod_name, external, outcome);
                }
                Err(err) => {
                    self.error_message = Some(format!("Container list task failed: {err}"));
                    self.status_message.clear();
                }
            },
            PendingOp::FetchServicePods {
                service_name,
                external,
                handle,
            } => match handle.await {
                Ok(outcome) => {
                    self.apply_fetch_service_pods(service_name, external, outcome);
                }
                Err(err) => {
                    self.error_message = Some(format!("Service pod list task failed: {err}"));
                    self.status_message.clear();
                }
            },
            PendingOp::Refresh { handle } => match handle.await {
                Ok(outcome) => {
                    self.apply_refresh(outcome).await;
                }
                Err(err) => {
                    self.error_message = Some(format!("Refresh task failed: {err}"));
                    self.status_message.clear();
                }
            },
        }
    }

    /// Drain a pending log-open request set by a finished fetch op.
    pub async fn flush_pending_log_open(&mut self) {
        let Some(req) = self.pending_open_log.take() else {
            return;
        };
        match req {
            PendingLogOpen::Pod {
                pod_name,
                container,
            } => {
                self.open_log_view(pod_name, container).await;
            }
            PendingLogOpen::MultiPod { title, pod_names } => {
                self.open_multi_pod_log_view(title, pod_names).await;
            }
        }
    }

    pub async fn refresh(&mut self) {
        self.spawn_full_refresh().await;
    }

    pub async fn spawn_full_refresh(&mut self) {
        if self.busy_soft_refuse() {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let kind = self.active_kind;
        let context = self.active_context.clone();
        self.status_message = "Refreshing…".into();
        self.error_message = None;
        let handle = tokio::spawn(async move { refresh_work(manager, kind, context).await });
        self.pending_op = Some(PendingOp::Refresh { handle });
    }

    async fn apply_refresh(&mut self, outcome: RefreshOutcome) {
        match outcome {
            Ok(result) => {
                self.contexts = result.contexts;
                self.context_index = self
                    .contexts
                    .iter()
                    .position(|c| c == self.active_context.as_str())
                    .unwrap_or(0);
                self.update_namespace_cache(&self.active_context.clone(), result.namespaces);
                self.namespace_index = result.namespace_index;
                if let Some(ns) = self.namespaces.get(result.namespace_index) {
                    self.active_namespace = ns.clone();
                }
                self.rows = result.rows;
                if self.selected >= self.rows.len() {
                    self.selected = self.rows.len().saturating_sub(1);
                }
                self.clamp_table_selection();
                self.error_message = None;
                self.update_status_from_rows();
            }
            Err(err) => {
                self.error_message = Some(err.user_message());
                self.status_message.clear();
            }
        }
    }

    /// Lightweight row reload for mutating actions (delete, scale, etc.).
    pub async fn reload_rows(&mut self) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let kind = self.active_kind;
        if kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
            let idx = self
                .selected_crd_index
                .min(self.crd_targets.len().saturating_sub(1));
            let target = self.crd_targets[idx].clone();
            let mut guard = manager.write().await;
            guard.set_selected_crd(Some(target));
            drop(guard);
        }
        if kind.uses_watch() {
            match manager.read().await.list_rows(kind).await {
                Ok(rows) => {
                    self.rows = rows;
                    self.error_message = None;
                    self.update_status_from_rows();
                }
                Err(err) => {
                    self.rows.clear();
                    self.error_message = Some(err.user_message());
                }
            }
            if self.selected >= self.rows.len() {
                self.selected = self.rows.len().saturating_sub(1);
            }
            self.clamp_table_selection();
        } else {
            // Non-watch: invalidate cache so the background fetch is authoritative.
            let key = self.kind_cache_key(kind);
            self.kind_row_cache.remove(&key);
            self.load_kind_rows_async().await;
        }
    }

    pub fn poll_snapshots(&mut self) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let kind = self.active_kind;
        if !kind.uses_watch() {
            return;
        }
        let Ok(guard) = manager.try_read() else {
            return;
        };
        let snapshot = guard.snapshot(kind);
        drop(guard);
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

    pub fn begin_screen_selection(&mut self, row: u16, col: u16) {
        self.detail_selection = None;
        self.table_selection = None;
        self.screen_selection = Some(TextSelection {
            start_line: row as usize,
            start_col: col as usize,
            end_line: row as usize,
            end_col: col as usize,
            dragging: true,
        });
    }

    pub fn update_screen_selection(&mut self, row: u16, col: u16) {
        let Some(sel) = self.screen_selection.as_mut() else {
            return;
        };
        if !sel.dragging {
            return;
        }
        sel.end_line = row as usize;
        sel.end_col = col as usize;
    }

    pub fn finish_screen_selection(&mut self) -> bool {
        let Some(sel) = self.screen_selection.as_mut() else {
            return false;
        };
        sel.dragging = false;
        let ((sl, sc), (el, ec)) = sel.normalized();
        if sl == el && sc == ec {
            self.screen_selection = None;
            return false;
        }
        if let Some(text) = self.selected_screen_text() {
            let trimmed = text.trim_end_matches('\n').to_string();
            if trimmed.chars().any(|c| !c.is_whitespace()) {
                match crate::clipboard::copy_text(&trimmed) {
                    Ok(()) => {
                        let preview = if trimmed.chars().count() > 40 {
                            format!("{}…", trimmed.chars().take(40).collect::<String>())
                        } else {
                            trimmed
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
        true
    }

    pub fn clear_screen_selection(&mut self) {
        self.screen_selection = None;
    }

    pub fn selected_screen_text(&self) -> Option<String> {
        let sel = self.screen_selection?;
        let ((sy, sx), (ey, ex)) = sel.normalized();
        if self.screen_cells.is_empty() {
            return None;
        }
        let ey = ey.min(self.screen_cells.len().saturating_sub(1));
        let mut out = String::new();
        for y in sy..=ey {
            let row = &self.screen_cells[y];
            if row.is_empty() {
                if y < ey {
                    out.push('\n');
                }
                continue;
            }
            let max_x = row.len().saturating_sub(1);
            let from = if y == sy { sx.min(max_x) } else { 0 };
            let to = if y == ey { ex.min(max_x) } else { max_x };
            if from <= to {
                for cell in &row[from..=to] {
                    out.push_str(cell);
                }
            }
            if y < ey {
                out.push('\n');
            }
        }
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
        let kinds: &[ResourceKind] = crate::ui::sidebar_kinds();
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
            .map(|l| l.text_width.max(1))
            .unwrap_or(40);
        let max_x = self.detail_max_scroll_x(width);
        if max_x == 0 {
            self.status_message = "Detail fits width — nothing to pan".into();
            return;
        }
        // One “page” step; keep at least 8 columns so a single keypress is obvious.
        let step = (width / 3).max(8) as i32;
        let next = self.detail_scroll_x as i32 + delta.signum() * step;
        self.detail_scroll_x = next.clamp(0, max_x as i32) as usize;
        self.status_message = format!("Detail pan {}/{}", self.detail_scroll_x, max_x);
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

    /// Pan detail horizontally. Works whenever the detail pane is open (focus optional).
    pub fn detail_pan_or_focus_left(&mut self) {
        if !self.detail_panel_visible() {
            self.focus_left();
            return;
        }
        self.focus = FocusPane::Detail;
        if self.detail_scroll_x > 0 {
            self.scroll_detail_x_by(-1);
        } else {
            self.focus_left();
        }
    }

    pub fn detail_pan_right(&mut self) {
        if !self.detail_panel_visible() {
            self.focus_right();
            return;
        }
        self.focus = FocusPane::Detail;
        self.scroll_detail_x_by(1);
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
            FocusPane::Table if self.detail_panel_visible() => FocusPane::Detail,
            FocusPane::Table => FocusPane::Table,
            FocusPane::Detail => FocusPane::Detail,
        };
    }

    pub async fn activate_sidebar_selection(&mut self) {
        let kinds: &[ResourceKind] = crate::ui::sidebar_kinds();
        if let Some(kind) = kinds.get(self.sidebar_index) {
            self.set_kind(*kind).await;
        }
    }

    pub async fn next_kind(&mut self) {
        let kinds: &[ResourceKind] = crate::ui::sidebar_kinds();
        let idx = kinds
            .iter()
            .position(|k| *k == self.active_kind)
            .unwrap_or(0);
        let next = (idx + 1) % kinds.len();
        self.set_kind(kinds[next]).await;
    }

    pub async fn prev_kind(&mut self) {
        let kinds: &[ResourceKind] = crate::ui::sidebar_kinds();
        let idx = kinds
            .iter()
            .position(|k| *k == self.active_kind)
            .unwrap_or(0);
        let prev = if idx == 0 { kinds.len() - 1 } else { idx - 1 };
        self.set_kind(kinds[prev]).await;
    }

    async fn set_kind(&mut self, kind: ResourceKind) {
        if self.exclusive_op_open() {
            self.busy_soft_refuse();
            return;
        }
        self.abort_lightweight_pending();
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

        if let Some(manager) = self.manager.clone() {
            let mut guard = manager.write().await;
            if let Err(err) = guard.set_active_kind(kind).await {
                self.error_message = Some(err.user_message());
            }
        }
        self.pull_rows_now().await;
    }

    async fn sync_context_list(&mut self) {
        if let Ok(contexts) = ClusterManager::list_contexts().await {
            self.contexts = contexts;
            self.context_index = self
                .contexts
                .iter()
                .position(|c| c == self.active_context.as_str())
                .unwrap_or(0);
            self.save_cluster_cache_async();
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
        if self.namespaces.is_empty() {
            let ctx = self.active_context.clone();
            self.sync_namespaces_for_context(&ctx);
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
            Some(Overlay::EditorPicker(state)) => self.filtered_editor_indices(&state.search),
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

    pub fn filtered_editor_indices(&self, search: &str) -> Vec<usize> {
        let query = search.to_lowercase();
        self.editor_candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                query.is_empty()
                    || c.label.to_lowercase().contains(&query)
                    || c.binary.to_lowercase().contains(&query)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Open the first-run editor picker. Caller decides whether to show it.
    pub fn show_editor_picker(&mut self) {
        self.overlay = Some(Overlay::EditorPicker(ListPickerState {
            search: String::new(),
            selected: 0,
        }));
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
            Some(Overlay::EditorPicker(_)) => self.filtered_editor_indices(search),
            _ => Vec::new(),
        }
    }

    fn overlay_list_state_mut(&mut self) -> Option<&mut ListPickerState> {
        match &mut self.overlay {
            Some(
                Overlay::Context(state)
                | Overlay::Namespace(state)
                | Overlay::Favorites(state)
                | Overlay::EditorPicker(state),
            ) => Some(state),
            Some(Overlay::Container { state, .. }) => Some(state),
            _ => None,
        }
    }

    fn overlay_search(&self) -> Option<String> {
        match &self.overlay {
            Some(
                Overlay::Context(state)
                | Overlay::Namespace(state)
                | Overlay::Favorites(state)
                | Overlay::EditorPicker(state),
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
            let max = 5i32;
            let cur = match cursor {
                SettingsCursor::NativePortForward => 0,
                SettingsCursor::ExternalLogs => 1,
                SettingsCursor::Theme => 2,
                SettingsCursor::AddKubeconfigPath => 3,
                SettingsCursor::ExtraKubeconfigList => 4,
                SettingsCursor::Editor => 5,
            };
            let next = (cur + delta).clamp(0, max);
            *cursor = match next {
                0 => SettingsCursor::NativePortForward,
                1 => SettingsCursor::ExternalLogs,
                2 => SettingsCursor::Theme,
                3 => SettingsCursor::AddKubeconfigPath,
                4 => SettingsCursor::ExtraKubeconfigList,
                _ => SettingsCursor::Editor,
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
        if let Some(Overlay::Input { value, cursor, .. }) = &mut self.overlay {
            if !ch.is_control() {
                let idx = value
                    .char_indices()
                    .nth(*cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(value.len());
                value.insert(idx, ch);
                *cursor += 1;
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
        if let Some(Overlay::Input { value, cursor, .. }) = &mut self.overlay {
            if *cursor > 0 {
                let idx = value
                    .char_indices()
                    .nth(*cursor - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(value.len());
                value.remove(idx);
                *cursor -= 1;
            }
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

    pub fn input_cursor_left(&mut self) {
        if let Some(Overlay::Input { cursor, .. }) = &mut self.overlay {
            *cursor = cursor.saturating_sub(1);
        }
    }

    pub fn input_cursor_right(&mut self) {
        if let Some(Overlay::Input { value, cursor, .. }) = &mut self.overlay {
            *cursor = (*cursor + 1).min(value.chars().count());
        }
    }

    pub fn input_cursor_home(&mut self) {
        if let Some(Overlay::Input { cursor, .. }) = &mut self.overlay {
            *cursor = 0;
        }
    }

    pub fn input_cursor_end(&mut self) {
        if let Some(Overlay::Input { value, cursor, .. }) = &mut self.overlay {
            *cursor = value.chars().count();
        }
    }

    pub fn input_delete_at_cursor(&mut self) {
        if let Some(Overlay::Input { value, cursor, .. }) = &mut self.overlay {
            if let Some((idx, _)) = value.char_indices().nth(*cursor) {
                value.remove(idx);
            }
        }
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
                purpose,
            } => {
                self.confirm_container_picker(pod_name, containers, state, purpose)
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
            Overlay::EditorPicker(state) => self.confirm_editor_picker(state).await,
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
        purpose: ContainerPickerPurpose,
    ) {
        let indices = filter_indices(&containers, &state.search);
        let Some(&container_idx) = indices.get(state.selected) else {
            return;
        };
        let container = containers[container_idx].clone();
        self.close_overlay();
        match purpose {
            ContainerPickerPurpose::Logs => {
                self.open_log_view(pod_name, Some(container)).await;
            }
            ContainerPickerPurpose::ExternalLogs => {
                self.spawn_external_pod_logs(&pod_name, Some(container.as_str()));
            }
            ContainerPickerPurpose::Exec => {
                self.pending_external = Some(ExternalRequest::ExecShell {
                    name: pod_name,
                    container: Some(container),
                });
            }
        }
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
        self.switch_to_context(context).await;
    }

    async fn confirm_namespace_picker(&mut self, state: ListPickerState) {
        let indices = self.filtered_namespace_indices(&state.search);
        let Some(&namespace_idx) = indices.get(state.selected) else {
            return;
        };
        let namespace = self.namespaces[namespace_idx].clone();
        self.close_overlay();
        self.switch_to_namespace(namespace).await;
    }

    async fn switch_to_context(&mut self, context: String) {
        if self.busy_soft_refuse() {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let kind = self.active_kind;
        let cached_namespaces = self.namespaces_by_context.get(&context).cloned();
        self.status_message = format!("Switching to {context}…");
        self.error_message = None;
        let ctx = context.clone();
        let handle = tokio::spawn(async move {
            switch_context_work(manager, &context, kind, cached_namespaces).await
        });
        self.pending_op = Some(PendingOp::SwitchContext {
            context: ctx,
            handle,
        });
    }

    async fn apply_switch_context(&mut self, context: &str, outcome: SwitchContextOutcome) {
        match outcome {
            Ok(result) => {
                self.context_index = self.contexts.iter().position(|c| c == context).unwrap_or(0);
                self.active_context = context.to_string();
                self.update_namespace_cache(context, result.namespaces);
                self.namespace_index = result.namespace_index;
                if let Some(manager) = self.manager.clone() {
                    let guard = manager.read().await;
                    self.active_namespace = guard.namespace().to_string();
                }
                self.crd_targets = result.crd_targets;
                self.selected_crd_index = 0;
                if self.active_kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
                    if let Some(manager) = self.manager.clone() {
                        let target = self.crd_targets[0].clone();
                        let mut guard = manager.write().await;
                        guard.set_selected_crd(Some(target));
                    }
                }
                self.selected = 0;
                self.clear_detail();
                self.error_message = None;
                if !self.cluster_tabs.iter().any(|t| t == context) {
                    self.cluster_tabs.push(context.to_string());
                }
                self.fire_plugins(context);
                self.persist_ui_settings();
                self.pull_rows_now().await;
                self.status_message = format!("{context} ready");
            }
            Err(err) => {
                self.error_message = Some(err.user_message());
                self.status_message.clear();
            }
        }
    }

    async fn switch_to_namespace(&mut self, namespace: String) {
        if self.busy_soft_refuse() {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let kind = self.active_kind;
        let ns = namespace.clone();
        self.status_message = format!("Switching to {ns}…");
        self.error_message = None;
        let handle =
            tokio::spawn(async move { switch_namespace_work(manager, &namespace, kind).await });
        self.pending_op = Some(PendingOp::SwitchNamespace {
            namespace: ns,
            handle,
        });
    }

    fn apply_switch_namespace(&mut self, namespace: &str, outcome: SwitchNamespaceOutcome) {
        match outcome {
            Ok(result) => {
                self.active_namespace = namespace.to_string();
                self.namespace_index = self
                    .namespaces
                    .iter()
                    .position(|n| n == namespace)
                    .unwrap_or(0);
                self.rows = result.rows;
                if self.selected >= self.rows.len() {
                    self.selected = self.rows.len().saturating_sub(1);
                }
                self.clamp_table_selection();
                self.selected = 0;
                self.clear_detail();
                self.error_message = None;
                self.status_message = format!("Switched to {namespace}");
            }
            Err(err) => {
                self.error_message = Some(err.user_message());
                self.status_message.clear();
            }
        }
    }

    pub async fn next_crd_target(&mut self) {
        if self.crd_targets.is_empty() {
            return;
        }
        self.selected_crd_index = (self.selected_crd_index + 1) % self.crd_targets.len();
        if let Some(manager) = self.manager.clone() {
            let target = self.crd_targets[self.selected_crd_index].clone();
            let mut guard = manager.write().await;
            guard.set_selected_crd(Some(target));
        }
        self.pull_rows_now().await;
    }

    pub fn set_detail_tab(&mut self, tab: DetailTab) {
        self.detail_tab = tab;
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.detail_selection = None;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
    }

    /// Right-hand Describe/Events/Metrics pane — hidden until `d` loads content.
    pub fn detail_panel_visible(&self) -> bool {
        self.detail_loading()
            || !self.detail_yaml.is_empty()
            || !self.detail_events.is_empty()
            || !self.detail_metrics.is_empty()
    }

    pub fn detail_loading(&self) -> bool {
        matches!(self.pending_op, Some(PendingOp::LoadDetail { .. }))
    }

    pub fn close_detail_panel(&mut self) {
        self.clear_detail();
    }

    fn clear_detail(&mut self) {
        self.detail_yaml.clear();
        self.detail_events.clear();
        self.detail_metrics.clear();
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.detail_selection = None;
        self.clear_detail_search();
        self.detail_layout = None;
        if self.focus == FocusPane::Detail {
            self.focus = FocusPane::Table;
        }
    }

    pub fn detail_search_active(&self) -> bool {
        self.detail_search_mode
    }

    pub fn detail_search_visible(&self) -> bool {
        self.detail_search_mode || !self.detail_search_query.is_empty()
    }

    pub fn enter_detail_search(&mut self) {
        if !self.detail_panel_visible() {
            return;
        }
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
            .map(|l| l.text_width.max(1))
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

    pub fn detail_contains_pos(&self, row: u16, column: u16) -> bool {
        let Some(layout) = self.detail_layout else {
            return false;
        };
        let area = layout.panel;
        column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    pub fn detail_hscroll_contains_pos(&self, row: u16, column: u16) -> bool {
        let Some(area) = self.detail_layout.and_then(|l| l.hscroll_area) else {
            return false;
        };
        column >= area.x
            && column < area.x.saturating_add(area.width)
            && row >= area.y
            && row < area.y.saturating_add(area.height)
    }

    pub async fn load_detail(&mut self) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let Some(row_idx) = self.selected_row_index() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let Some(row) = self.rows.get(row_idx).cloned() else {
            self.error_message = Some("No resource selected".into());
            return;
        };

        self.error_message = None;
        let kind = self.active_kind;
        let tab = self.detail_tab;
        self.status_message = format!("Loading {tab:?}…");
        let work_name = row.name.clone();
        let handle =
            tokio::spawn(async move { load_detail_work(manager, kind, &work_name, tab).await });
        self.pending_op = Some(PendingOp::LoadDetail { handle });
    }

    fn apply_load_detail(&mut self, outcome: LoadDetailOutcome) {
        match outcome {
            Ok(LoadDetailResult::Describe(yaml)) => {
                self.detail_yaml = yaml;
            }
            Ok(LoadDetailResult::Events(text)) => {
                self.detail_events = text;
            }
            Ok(LoadDetailResult::Metrics(text)) => {
                self.detail_metrics = text;
            }
            Err(err) => {
                let msg = err.user_message();
                match self.detail_tab {
                    DetailTab::Describe => self.detail_yaml = msg,
                    DetailTab::Events => self.detail_events = msg,
                    DetailTab::Metrics => self.detail_metrics = msg,
                }
            }
        }
        self.detail_scroll = 0;
        self.detail_scroll_x = 0;
        self.recompute_detail_matches();
        self.scroll_detail_to_current_match();
        self.focus = FocusPane::Detail;
        self.status_message.clear();
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
        let Some(row) = self.rows.get(row_idx).cloned() else {
            self.error_message = Some("No pod selected.".into());
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };

        let pod_name = row.name.clone();
        self.status_message = format!("Loading containers for {pod_name}…");
        self.error_message = None;
        let work_pod = pod_name.clone();
        let handle = tokio::spawn(async move {
            let guard = manager.read().await;
            guard.pod_containers(&work_pod).await
        });
        self.pending_op = Some(PendingOp::FetchContainers {
            pod_name,
            external: self.logs_external_for(&row.status),
            handle,
        });
    }

    fn apply_fetch_containers(
        &mut self,
        pod_name: String,
        external: bool,
        outcome: FetchContainersOutcome,
    ) {
        self.status_message.clear();
        match outcome {
            Ok(containers) if containers.len() > 1 => {
                let purpose = if external {
                    ContainerPickerPurpose::ExternalLogs
                } else {
                    ContainerPickerPurpose::Logs
                };
                self.overlay = Some(Overlay::Container {
                    pod_name,
                    containers: containers.into_iter().map(|c| c.name).collect(),
                    state: ListPickerState {
                        search: String::new(),
                        selected: 0,
                    },
                    purpose,
                });
            }
            Ok(containers) if external => {
                let container = containers.first().map(|c| c.name.clone());
                self.spawn_external_pod_logs(&pod_name, container.as_deref());
            }
            Ok(containers) => {
                let container = containers.first().map(|c| c.name.clone());
                self.pending_open_log = Some(PendingLogOpen::Pod {
                    pod_name,
                    container,
                });
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    /// Spawn `kubectl logs -f` in a new terminal emulator; fall back to the
    /// built-in log view when no emulator can be launched (e.g. over SSH).
    fn spawn_external_pod_logs(&mut self, pod_name: &str, container: Option<&str>) {
        match rl_core::spawn_kubectl_logs_terminal(
            &self.active_context,
            &self.active_namespace,
            pod_name,
            container,
        ) {
            Ok(_child) => {
                self.status_message = format!("Opened logs for {pod_name} in a new terminal");
            }
            Err(_) => {
                self.status_message = "No terminal emulator found — using built-in log view".into();
                self.pending_open_log = Some(PendingLogOpen::Pod {
                    pod_name: pod_name.to_string(),
                    container: container.map(str::to_string),
                });
            }
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
        let Some(row) = self.rows.get(row_idx).cloned() else {
            self.error_message = Some("No service selected.".into());
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };

        let service_name = row.name.clone();
        self.status_message = format!("Finding pods for {service_name}…");
        self.error_message = None;
        let work_svc = service_name.clone();
        let handle = tokio::spawn(async move {
            let guard = manager.read().await;
            guard.pods_for_service(&work_svc).await
        });
        self.pending_op = Some(PendingOp::FetchServicePods {
            service_name,
            external: self.external_logs,
            handle,
        });
    }

    fn apply_fetch_service_pods(
        &mut self,
        service_name: String,
        external: bool,
        outcome: FetchServicePodsOutcome,
    ) {
        self.status_message.clear();
        match outcome {
            Ok(info) if info.names.is_empty() => {
                self.error_message = Some(format!(
                    "No pods match service `{service_name}` (missing or empty selector?)."
                ));
            }
            Ok(info) => {
                let selector = info.selector;
                let mut pods = info.names;
                // `-l` matches every pod, so kubectl needs headroom for all of them.
                let total_pods = pods.len();
                const MAX_SERVICE_LOG_PODS: usize = 40;
                if pods.len() > MAX_SERVICE_LOG_PODS {
                    let dropped = pods.len() - MAX_SERVICE_LOG_PODS;
                    pods.truncate(MAX_SERVICE_LOG_PODS);
                    self.status_message = format!(
                        "Service has many pods; streaming first {MAX_SERVICE_LOG_PODS} (+{dropped} skipped)."
                    );
                }
                if external {
                    if let Some(selector) = selector.as_deref() {
                        match rl_core::spawn_kubectl_selector_logs_terminal(
                            &self.active_context,
                            &self.active_namespace,
                            selector,
                            total_pods,
                        ) {
                            Ok(_child) => {
                                self.status_message = format!(
                                    "Opened logs for svc/{service_name} ({total_pods} pods) in a new terminal"
                                );
                                return;
                            }
                            Err(_) => {
                                self.status_message =
                                    "No terminal emulator found — using built-in log view".into();
                            }
                        }
                    }
                }
                let title = format!("svc/{service_name} ({} pods)", pods.len());
                self.pending_open_log = Some(PendingLogOpen::MultiPod {
                    title,
                    pod_names: pods,
                });
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn open_log_view(&mut self, pod_name: String, container: Option<String>) {
        self.close_log_view();
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        let (line_tx, line_rx) = mpsc::channel(8_192);
        let (err_tx, err_rx) = mpsc::channel(8);
        let stream_task =
            guard.spawn_log_stream(pod_name.clone(), container.clone(), true, line_tx, err_tx);
        drop(guard);

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
            focus_wrapped_row: None,
            line_rx,
            err_rx,
            stream_task,
        });
        self.error_message = None;
    }

    async fn open_multi_pod_log_view(&mut self, title: String, pod_names: Vec<String>) {
        self.close_log_view();
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        let (line_tx, line_rx) = mpsc::channel(8_192);
        let (err_tx, err_rx) = mpsc::channel(8);
        let stream_task = guard.spawn_multi_pod_log_stream(pod_names, true, line_tx, err_tx);
        drop(guard);

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
            focus_wrapped_row: None,
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
        if let Some(log) = self.log_view.as_mut() {
            log.focus_wrapped_row = Some(wrapped_row);
        }
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
                // Still mark the row so the user sees which line was targeted.
                log.highlight_source = Some(source);
                self.error_message = Some(format!("Clipboard unavailable: {err}"));
            }
        }
    }

    /// Copy the focused / highlighted log line (or the top visible row as fallback).
    pub fn yank_focused_log_line(&mut self) {
        let Some(log) = self.log_view.as_ref() else {
            return;
        };
        let wrapped = log
            .focus_wrapped_row
            .or_else(|| {
                log.highlight_source
                    .and_then(|src| log.wrapped_sources.iter().position(|&s| s == src))
            })
            .unwrap_or(log.scroll);
        self.yank_log_line_at_wrapped_row(wrapped);
    }

    pub fn set_log_focus_wrapped_row(&mut self, wrapped_row: usize) {
        if let Some(log) = self.log_view.as_mut() {
            log.focus_wrapped_row = Some(wrapped_row);
            log.highlight_source = Some(log.source_line_for_wrapped_row(wrapped_row));
            log.follow = false;
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

/// Network work for a context switch, run off the input loop.
async fn switch_context_work(
    manager: SharedManager,
    context: &str,
    kind: ResourceKind,
    cached_namespaces: Option<Vec<String>>,
) -> SwitchContextOutcome {
    let mut guard = manager.write().await;
    guard.switch_context(context, kind).await?;
    let namespaces = if let Some(cached) = cached_namespaces.filter(|n| !n.is_empty()) {
        cached
    } else {
        guard.list_namespaces().await.unwrap_or_default()
    };
    let namespace_index = namespaces
        .iter()
        .position(|n| n == guard.namespace())
        .unwrap_or(0);
    let crd_targets = guard.crd_targets().to_vec();
    Ok(SwitchContextResult {
        namespaces,
        namespace_index,
        crd_targets,
    })
}

async fn load_detail_work(
    manager: SharedManager,
    kind: ResourceKind,
    name: &str,
    tab: DetailTab,
) -> LoadDetailOutcome {
    let guard = manager.read().await;
    match tab {
        DetailTab::Describe => Ok(LoadDetailResult::Describe(
            guard.resource_yaml(kind, name).await?,
        )),
        DetailTab::Events => {
            let events = guard.resource_events(kind, name).await?;
            Ok(LoadDetailResult::Events(format_events_text(&events)))
        }
        DetailTab::Metrics => {
            if kind != ResourceKind::Pod {
                return Ok(LoadDetailResult::Metrics(
                    "Metrics are only available for pods (requires metrics-server).".into(),
                ));
            }
            let metrics = guard.pod_metrics().await?;
            let filtered: Vec<_> = metrics.into_iter().filter(|m| m.pod_name == name).collect();
            if filtered.is_empty() {
                Ok(LoadDetailResult::Metrics(
                    "No metrics for this pod (is metrics-server installed?)".into(),
                ))
            } else {
                Ok(LoadDetailResult::Metrics(format_metrics_text(&filtered)))
            }
        }
    }
}

async fn switch_namespace_work(
    manager: SharedManager,
    namespace: &str,
    kind: ResourceKind,
) -> SwitchNamespaceOutcome {
    let mut guard = manager.write().await;
    guard.set_namespace(namespace.to_string(), kind).await?;
    let rows = guard.list_rows(kind).await.unwrap_or_default();
    Ok(SwitchNamespaceResult { rows })
}

async fn refresh_work(
    manager: SharedManager,
    kind: ResourceKind,
    context: String,
) -> RefreshOutcome {
    let contexts = ClusterManager::list_contexts().await?;
    let mut guard = manager.write().await;
    if kind.uses_watch() {
        guard.refresh_watch(kind).await?;
    }
    let namespaces = guard.list_namespaces().await.unwrap_or_default();
    let namespace_index = namespaces
        .iter()
        .position(|n| n == guard.namespace())
        .unwrap_or(0);
    if kind == ResourceKind::Crd {
        // Caller may set CRD target separately; list_rows handles empty selection.
    }
    let rows = guard.list_rows(kind).await.unwrap_or_default();
    let _ = context;
    Ok(RefreshResult {
        contexts,
        namespaces,
        namespace_index,
        rows,
    })
}

#[cfg(test)]
mod log_target_tests {
    use super::TuiApp;

    #[test]
    fn terminated_pods_skip_external_log_terminal() {
        let mut app = TuiApp::new();
        app.external_logs = true;
        assert!(!app.logs_external_for("Succeeded"));
        assert!(!app.logs_external_for("Failed"));
        assert!(app.logs_external_for("Running"));
        assert!(app.logs_external_for("Pending"));
        app.external_logs = false;
        assert!(!app.logs_external_for("Running"));
    }
}
