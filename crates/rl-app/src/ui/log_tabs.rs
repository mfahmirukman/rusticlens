use rl_core::ContainerInfo;

use crate::{log_debug, log_info};

/// Hard cap on raw log text kept per tab (~1 MiB).
pub const MAX_BYTES_PER_TAB: usize = 1_048_576;
/// Secondary cap on line count per tab.
pub const MAX_LINES_PER_TAB: usize = 2_000;
/// Emit a memory sample every N live-stream lines per tab.
const LOG_MEMORY_SAMPLE_LINES: usize = 500;

#[derive(Debug, Clone)]
pub struct LogTab {
    pub id: u64,
    pub context: String,
    pub pod_name: String,
    pub namespace: String,
    pub container: Option<String>,
    pub containers: Vec<ContainerInfo>,
    lines: Vec<String>,
    pub has_more_older: bool,
    pub loading_older: bool,
    /// Previous frame scroll offset — used to detect scrolling up into the top edge.
    pub(crate) last_scroll_offset_y: Option<f32>,
    /// Re-armed when leaving the top; used when content fits without scrolling (wheel-up).
    pub(crate) older_fetch_armed: bool,
    /// After prepending older lines, bump scroll offset by this many rows once.
    pub scroll_compensate_rows: usize,
    /// Virtual row index to scroll into view (log search navigation).
    pub scroll_to_match_row: Option<usize>,
    trimmed_lines: u64,
    /// Highest line-count milestone already logged (`1`, `100`, `500`, …).
    logged_milestone: usize,
}

impl LogTab {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn byte_len(&self) -> usize {
        self.lines.iter().map(|l| l.len() + 1).sum()
    }

    fn metadata_bytes(&self) -> usize {
        self.context.len()
            + self.pod_name.len()
            + self.namespace.len()
            + self.container.as_ref().map(|s| s.len()).unwrap_or(0)
    }

    fn memory_bytes(&self) -> usize {
        self.byte_len() + self.metadata_bytes()
    }

    fn append_line(&mut self, line: String) {
        self.lines.push(line);
        self.trim_to_limits();
    }

    fn prepend_lines(&mut self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let mut merged = lines.to_vec();
        merged.append(&mut self.lines);
        self.lines = merged;
        self.trim_to_limits();
    }

    fn trim_to_limits(&mut self) {
        while self.lines.len() > MAX_LINES_PER_TAB {
            self.lines.remove(0);
            self.trimmed_lines += 1;
        }
        while self.byte_len() > MAX_BYTES_PER_TAB && !self.lines.is_empty() {
            self.lines.remove(0);
            self.trimmed_lines += 1;
        }
    }

    /// Line indices matching `filter` (empty filter = all lines).
    pub fn matching_indices(&self, filter: &str) -> Vec<usize> {
        if filter.is_empty() {
            return Vec::new();
        }
        let needle = filter.to_lowercase();
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct LogTabsState {
    tabs: Vec<LogTab>,
    active_id: Option<u64>,
    next_id: u64,
}

impl LogTabsState {
    pub fn active_id(&self) -> Option<u64> {
        self.active_id
    }

    pub fn tabs(&self) -> &[LogTab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<&LogTab> {
        self.active_id
            .and_then(|id| self.tabs.iter().find(|t| t.id == id))
    }

    pub fn tab_mut(&mut self, id: u64) -> Option<&mut LogTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Open or focus a tab for this pod. Returns tab id.
    pub fn open_tab(
        &mut self,
        context: String,
        pod_name: String,
        namespace: String,
        container: Option<String>,
    ) -> u64 {
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|t| t.context == context && t.pod_name == pod_name && t.namespace == namespace)
            .map(|t| t.id)
        {
            self.active_id = Some(existing);
            if let Some(tab) = self.tab_mut(existing) {
                if let Some(c) = container {
                    tab.container = Some(c);
                }
            }
            log_debug!(
                tab_id = existing,
                pod = %pod_name,
                "reused existing log tab"
            );
            return existing;
        }

        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(LogTab {
            id,
            context: context.clone(),
            pod_name: pod_name.clone(),
            namespace: namespace.clone(),
            container,
            containers: Vec::new(),
            lines: Vec::new(),
            has_more_older: true,
            loading_older: false,
            last_scroll_offset_y: None,
            older_fetch_armed: false,
            scroll_compensate_rows: 0,
            scroll_to_match_row: None,
            trimmed_lines: 0,
            logged_milestone: 0,
        });
        self.active_id = Some(id);
        log_info!(
            tab_id = id,
            pod = %pod_name,
            namespace = %namespace,
            tab_count = self.tabs.len(),
            max_kb_per_tab = MAX_BYTES_PER_TAB / 1024,
            "opened log tab"
        );
        self.log_memory_usage("tab_opened");
        id
    }

    pub fn set_active(&mut self, id: u64) {
        if self.tabs.iter().any(|t| t.id == id) {
            self.active_id = Some(id);
        }
    }

    /// Remove tab and return its id so the backend can stop the stream.
    pub fn close_tab(&mut self, id: u64) -> Option<u64> {
        let idx = self.tabs.iter().position(|t| t.id == id)?;
        let removed = self.tabs.remove(idx);
        if self.active_id == Some(id) {
            self.active_id = self.tabs.last().map(|t| t.id);
        }
        log_info!(
            tab_id = removed.id,
            pod = %removed.pod_name,
            freed_lines = removed.line_count(),
            freed_bytes = removed.memory_bytes(),
            freed_kb = removed.memory_bytes() / 1024,
            remaining_tabs = self.tabs.len(),
            "closed log tab"
        );
        self.log_memory_usage("tab_closed");
        Some(id)
    }

    pub fn push_line(&mut self, tab_id: u64, line: String) {
        let (log_tail_milestone, trimmed_now, sample_line) = {
            let Some(tab) = self.tab_mut(tab_id) else {
                return;
            };
            let line_bytes = line.len();
            let trimmed_before = tab.trimmed_lines;
            tab.append_line(line);
            let mut log_tail_milestone = false;
            let n = tab.line_count();
            if n == 1 {
                log_info!(
                    tab_id,
                    pod = %tab.pod_name,
                    "log stream started receiving lines"
                );
            } else if n >= 100 && tab.logged_milestone < 100 {
                tab.logged_milestone = 100;
                log_info!(
                    tab_id,
                    pod = %tab.pod_name,
                    tab_lines = n,
                    tab_bytes = tab.byte_len(),
                    tab_kb = tab.byte_len() / 1024,
                    "initial log batch held in app buffer (compare tab_kb to process RSS)"
                );
            } else if n >= 500 && tab.logged_milestone < 500 {
                tab.logged_milestone = 500;
                log_tail_milestone = true;
                log_info!(
                    tab_id,
                    pod = %tab.pod_name,
                    tab_lines = n,
                    tab_bytes = tab.byte_len(),
                    tab_kb = tab.byte_len() / 1024,
                    "typical initial tail loaded — app buffer size"
                );
            }
            let trimmed_now = tab.trimmed_lines > trimmed_before;
            let sample_line = if tab.line_count().is_multiple_of(LOG_MEMORY_SAMPLE_LINES) {
                Some((
                    tab.pod_name.clone(),
                    tab.line_count(),
                    tab.byte_len(),
                    line_bytes,
                ))
            } else {
                None
            };
            (log_tail_milestone, trimmed_now, sample_line)
        };

        if log_tail_milestone {
            self.log_memory_usage("initial_tail_loaded");
        }
        if trimmed_now {
            let Some(tab) = self.tab_mut(tab_id) else {
                return;
            };
            log_info!(
                tab_id,
                pod = %tab.pod_name,
                trimmed_total = tab.trimmed_lines,
                tab_lines = tab.line_count(),
                tab_bytes = tab.byte_len(),
                tab_kb = tab.byte_len() / 1024,
                max_kb = MAX_BYTES_PER_TAB / 1024,
                "trimmed log buffer"
            );
            self.log_memory_usage("buffer_trimmed");
        } else if let Some((pod_name, tab_lines, tab_bytes, line_bytes)) = sample_line {
            log_debug!(
                tab_id,
                pod = %pod_name,
                tab_lines,
                tab_bytes,
                tab_kb = tab_bytes / 1024,
                last_line_bytes = line_bytes,
                "live log stream memory sample"
            );
        }
    }

    pub fn clear_lines(&mut self, tab_id: u64) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        let cleared_lines = tab.line_count();
        let cleared_bytes = tab.memory_bytes();
        tab.lines.clear();
        tab.trimmed_lines = 0;
        tab.logged_milestone = 0;
        tab.has_more_older = true;
        tab.loading_older = false;
        tab.last_scroll_offset_y = None;
        tab.older_fetch_armed = false;
        log_info!(
            tab_id,
            pod = %tab.pod_name,
            cleared_lines,
            cleared_bytes,
            cleared_kb = cleared_bytes / 1024,
            "cleared log tab buffer"
        );
        self.log_memory_usage("buffer_cleared");
    }

    pub fn set_loading_older(&mut self, tab_id: u64, loading: bool) {
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.loading_older = loading;
            if loading {
                log_debug!(
                    tab_id,
                    pod = %tab.pod_name,
                    tab_lines = tab.line_count(),
                    tab_bytes = tab.byte_len(),
                    "fetching older logs"
                );
            }
        }
    }

    pub fn apply_older_logs(&mut self, tab_id: u64, prepended: Vec<String>, has_more: bool) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        let loaded_lines = prepended.len();
        let loaded_bytes: usize = prepended.iter().map(|l| l.len()).sum();
        let lines_before = tab.line_count();
        let bytes_before = tab.byte_len();

        tab.prepend_lines(&prepended);
        tab.has_more_older = has_more;
        tab.loading_older = false;
        tab.scroll_compensate_rows = prepended.len();

        log_info!(
            tab_id,
            pod = %tab.pod_name,
            loaded_lines,
            loaded_bytes,
            loaded_kb = loaded_bytes / 1024,
            lines_before,
            lines_after = tab.line_count(),
            bytes_before,
            bytes_after = tab.byte_len(),
            delta_bytes = tab.byte_len().saturating_sub(bytes_before),
            delta_kb = tab.byte_len().saturating_sub(bytes_before) / 1024,
            has_more_older = has_more,
            "older logs loaded into memory"
        );
        self.log_memory_usage("older_logs_applied");
    }

    pub fn set_containers(&mut self, tab_id: u64, containers: Vec<ContainerInfo>) {
        let Some(tab) = self.tab_mut(tab_id) else {
            return;
        };
        if tab.container.is_none() {
            tab.container = containers
                .iter()
                .find(|c| c.ready)
                .map(|c| c.name.clone())
                .or_else(|| containers.first().map(|c| c.name.clone()));
        }
        tab.containers = containers;
    }

    fn log_memory_usage(&self, reason: &str) {
        let total_lines: usize = self.tabs.iter().map(|t| t.line_count()).sum();
        let total_bytes: usize = self.tabs.iter().map(|t| t.byte_len()).sum();
        let total_with_meta: usize = self.tabs.iter().map(|t| t.memory_bytes()).sum();
        log_info!(
            reason,
            tab_count = self.tabs.len(),
            total_lines,
            total_bytes,
            total_kb = total_bytes / 1024,
            total_mb = total_bytes / (1024 * 1024),
            cap_kb_per_tab = MAX_BYTES_PER_TAB / 1024,
            cap_lines_per_tab = MAX_LINES_PER_TAB,
            "log buffer memory usage"
        );
        for tab in &self.tabs {
            log_debug!(
                tab_id = tab.id,
                pod = %tab.pod_name,
                lines = tab.line_count(),
                bytes = tab.byte_len(),
                kb = tab.byte_len() / 1024,
                trimmed_lines = tab.trimmed_lines,
                loading_older = tab.loading_older,
                has_more_older = tab.has_more_older,
                "log tab memory"
            );
        }
        log_debug!(
            reason,
            total_with_meta_kb = total_with_meta / 1024,
            "log buffer memory including tab metadata"
        );
    }
}
