use ratatui::layout::Rect;

use rl_core::{
    config::context_is_usable, format_events_text, format_metrics_text, ClusterManager, CrdTarget,
    ResourceKind, ResourceRow, LOG_BUFFER_MAX_LINES,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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

#[derive(Debug, Clone)]
pub struct ListPickerState {
    pub search: String,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub enum Overlay {
    ContextPicker(ListPickerState),
    NamespacePicker(ListPickerState),
    ContainerPicker {
        pod_name: String,
        containers: Vec<String>,
        state: ListPickerState,
    },
}

pub struct LogView {
    pub pod_name: String,
    pub container: Option<String>,
    pub lines: Vec<String>,
    pub scroll: usize,
    pub follow: bool,
    pub visible_lines: usize,
    pub wrap_width: usize,
    pub search_mode: bool,
    pub search_query: String,
    pub match_rows: Vec<usize>,
    pub match_cursor: usize,
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

pub struct TuiApp {
    pub connection: ConnectionState,
    pub manager: Option<ClusterManager>,
    pub active_kind: ResourceKind,
    pub sidebar_index: usize,
    pub focus: FocusPane,
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
    pub status_message: String,
    pub error_message: Option<String>,
    pub crd_targets: Vec<CrdTarget>,
    pub selected_crd_index: usize,
    pub overlay: Option<Overlay>,
    pub log_view: Option<LogView>,
    pub log_layout: Option<LogViewLayout>,
    pub table_filter: String,
    pub search_mode: bool,
    connect_attempted: bool,
}

impl TuiApp {
    pub fn connect_attempted(&self) -> bool {
        self.connect_attempted
    }

    pub fn new() -> Self {
        Self {
            connection: ConnectionState::Disconnected,
            manager: None,
            active_kind: ResourceKind::Pod,
            sidebar_index: 0,
            focus: FocusPane::Table,
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
            status_message: String::new(),
            error_message: None,
            crd_targets: Vec::new(),
            selected_crd_index: 0,
            overlay: None,
            log_view: None,
            log_layout: None,
            table_filter: String::new(),
            search_mode: false,
            connect_attempted: false,
        }
    }

    pub fn enter_search_mode(&mut self) {
        if self.active_kind != ResourceKind::Pod {
            return;
        }
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
        if self.active_kind != ResourceKind::Pod || self.table_filter.is_empty() {
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
    }

    pub fn poll_log_lines(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        while let Ok(line) = log.line_rx.try_recv() {
            log.push_line(line);
        }
        while let Ok(err) = log.err_rx.try_recv() {
            log.error = Some(err);
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
                self.manager = Some(manager);
                self.sync_context_list().await;
                self.connection = ConnectionState::Connected;
                self.status_message = "Connected".into();
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
    }

    fn move_sidebar(&mut self, delta: i32) {
        let kinds: Vec<ResourceKind> = ResourceKind::ALL.to_vec();
        let current = self.sidebar_index.min(kinds.len().saturating_sub(1));
        let next = (current as i32 + delta).clamp(0, kinds.len() as i32 - 1) as usize;
        self.sidebar_index = next;
    }

    fn scroll_detail(&mut self, delta: i32) {
        let content = match self.detail_tab {
            DetailTab::Describe => &self.detail_yaml,
            DetailTab::Events => &self.detail_events,
            DetailTab::Metrics => &self.detail_metrics,
        };
        let max_scroll = content.lines().count().saturating_sub(1) as i32;
        let next = self.detail_scroll as i32 + delta;
        self.detail_scroll = next.clamp(0, max_scroll) as u16;
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
        self.table_filter.clear();
        self.search_mode = false;
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
        self.overlay = Some(Overlay::ContextPicker(ListPickerState {
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
        self.overlay = Some(Overlay::NamespacePicker(ListPickerState {
            search: String::new(),
            selected,
        }));
    }

    pub fn picker_indices(&self) -> Vec<usize> {
        match &self.overlay {
            Some(Overlay::ContextPicker(state)) => self.filtered_context_indices(&state.search),
            Some(Overlay::NamespacePicker(state)) => self.filtered_namespace_indices(&state.search),
            Some(Overlay::ContainerPicker {
                state, containers, ..
            }) => filter_indices(containers, &state.search),
            None => Vec::new(),
        }
    }

    fn filtered_context_indices(&self, search: &str) -> Vec<usize> {
        filter_indices(&self.contexts, search)
    }

    fn filtered_namespace_indices(&self, search: &str) -> Vec<usize> {
        filter_indices(&self.namespaces, search)
    }

    fn picker_filtered_indices(&self, search: &str) -> Vec<usize> {
        match &self.overlay {
            Some(Overlay::ContextPicker(_)) => self.filtered_context_indices(search),
            Some(Overlay::NamespacePicker(_)) => self.filtered_namespace_indices(search),
            Some(Overlay::ContainerPicker { containers, .. }) => filter_indices(containers, search),
            None => Vec::new(),
        }
    }

    fn overlay_list_state_mut(&mut self) -> Option<&mut ListPickerState> {
        match &mut self.overlay {
            Some(Overlay::ContextPicker(state) | Overlay::NamespacePicker(state)) => Some(state),
            Some(Overlay::ContainerPicker { state, .. }) => Some(state),
            None => None,
        }
    }

    fn overlay_search(&self) -> Option<String> {
        match &self.overlay {
            Some(Overlay::ContextPicker(state) | Overlay::NamespacePicker(state)) => {
                Some(state.search.clone())
            }
            Some(Overlay::ContainerPicker { state, .. }) => Some(state.search.clone()),
            None => None,
        }
    }

    pub fn picker_move(&mut self, delta: i32) {
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
            Overlay::ContextPicker(state) => self.confirm_context_picker(state).await,
            Overlay::NamespacePicker(state) => self.confirm_namespace_picker(state).await,
            Overlay::ContainerPicker {
                pod_name,
                containers,
                state,
            } => {
                self.confirm_container_picker(pod_name, containers, state)
                    .await
            }
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
    }

    fn clear_detail(&mut self) {
        self.detail_yaml.clear();
        self.detail_events.clear();
        self.detail_metrics.clear();
        self.detail_scroll = 0;
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
                    return;
                }
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
        self.detail_scroll = 0;
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
                self.overlay = Some(Overlay::ContainerPicker {
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

    async fn open_log_view(&mut self, pod_name: String, container: Option<String>) {
        self.close_log_view();
        let Some(manager) = self.manager.as_ref() else {
            return;
        };

        let (line_tx, line_rx) = mpsc::channel(512);
        let (err_tx, err_rx) = mpsc::channel(8);
        let stream_task =
            manager.spawn_log_stream(pod_name.clone(), container.clone(), true, line_tx, err_tx);

        self.log_view = Some(LogView {
            pod_name,
            container,
            lines: Vec::new(),
            scroll: 0,
            follow: true,
            visible_lines: 1,
            wrap_width: 80,
            search_mode: false,
            search_query: String::new(),
            match_rows: Vec::new(),
            match_cursor: 0,
            error: None,
            highlight_source: None,
            line_rx,
            err_rx,
            stream_task,
        });
        self.error_message = None;
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
        log.wrap_width = wrap_width.max(1);
        log.recompute_matches();
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

    pub fn exit_log_search(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_mode = false;
        log.recompute_matches();
        if !log.search_query.is_empty() {
            log.scroll_to_current_match();
        }
    }

    pub fn clear_log_search(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.clear();
        log.match_rows.clear();
        log.match_cursor = 0;
        log.search_mode = false;
    }

    pub fn log_search_push_char(&mut self, ch: char) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.push(ch);
        log.recompute_matches();
        log.scroll_to_current_match();
    }

    pub fn log_search_backspace(&mut self) {
        let Some(log) = self.log_view.as_mut() else {
            return;
        };
        log.search_query.pop();
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
        match arboard::Clipboard::new().and_then(|mut clip| clip.set_text(text)) {
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
    pub fn wrapped_lines(&self) -> Vec<String> {
        wrap_log_lines(&self.lines, self.wrap_width)
    }

    pub fn source_line_for_wrapped_row(&self, wrapped_row: usize) -> usize {
        let width = self.wrap_width.max(1);
        let mut idx = 0;
        for (source_i, line) in self.lines.iter().enumerate() {
            let segs = wrapped_segment_count(line, width);
            if wrapped_row < idx + segs {
                return source_i;
            }
            idx += segs;
        }
        self.lines.len().saturating_sub(1)
    }

    fn max_scroll(&self) -> usize {
        self.wrapped_lines()
            .len()
            .saturating_sub(self.visible_lines)
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    fn recompute_matches(&mut self) {
        self.match_rows = find_log_match_rows(&self.wrapped_lines(), &self.search_query);
        if self.match_cursor >= self.match_rows.len() {
            self.match_cursor = 0;
        }
    }

    fn scroll_to_current_match(&mut self) {
        let Some(&row) = self.match_rows.get(self.match_cursor) else {
            return;
        };
        self.follow = false;
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

    fn push_line(&mut self, line: String) {
        self.lines.push(line);
        if self.lines.len() > LOG_BUFFER_MAX_LINES {
            let excess = self.lines.len() - LOG_BUFFER_MAX_LINES;
            self.lines.drain(0..excess);
            self.scroll = self.scroll.min(self.max_scroll());
        }
        if self.follow {
            self.scroll_to_bottom();
        }
    }
}

fn wrap_log_lines(lines: &[String], width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let end = (start + width).min(chars.len());
            out.push(chars[start..end].iter().collect());
            start = end;
        }
    }
    out
}

fn wrapped_segment_count(line: &str, width: usize) -> usize {
    if line.is_empty() {
        1
    } else {
        (line.chars().count() + width - 1) / width
    }
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
