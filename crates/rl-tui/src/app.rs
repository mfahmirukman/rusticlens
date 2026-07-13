use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use rl_core::{ClusterManager, ResourceKind, ResourceRow};

pub struct TuiApp {
    manager: ClusterManager,
    kinds: Vec<ResourceKind>,
    kind_index: usize,
    rows: Vec<ResourceRow>,
    selected: usize,
    detail: String,
    status: String,
    namespaces: Vec<String>,
    namespace_index: usize,
}

impl TuiApp {
    pub async fn new() -> Self {
        let manager = ClusterManager::connect_default(ResourceKind::Pod)
            .await
            .expect("connect to cluster");
        let namespaces = manager.list_namespaces().await.unwrap_or_default();
        let namespace_index = namespaces
            .iter()
            .position(|n| n == manager.namespace())
            .unwrap_or(0);
        let kinds = vec![
            ResourceKind::Pod,
            ResourceKind::Deployment,
            ResourceKind::Service,
            ResourceKind::ConfigMap,
            ResourceKind::Namespace,
        ];
        let mut app = Self {
            manager,
            kinds,
            kind_index: 0,
            rows: Vec::new(),
            selected: 0,
            detail: String::from("Press d to describe, r to refresh, Tab to switch kind."),
            status: String::new(),
            namespaces,
            namespace_index,
        };
        app.refresh().await;
        app
    }

    pub fn current_kind(&self) -> ResourceKind {
        self.kinds[self.kind_index]
    }

    pub async fn refresh(&mut self) {
        let kind = self.current_kind();
        self.rows = self
            .manager
            .list_rows(kind)
            .await
            .unwrap_or_default();
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.status = format!(
            "{} / {} — {} {}",
            self.manager.context(),
            self.manager.namespace(),
            kind.label(),
            self.rows.len()
        );
    }

    pub async fn poll_snapshots(&mut self) {
        let kind = self.current_kind();
        if matches!(kind, ResourceKind::HelmRelease | ResourceKind::Crd) {
            return;
        }
        let snapshot = self.manager.snapshot(kind);
        if snapshot.rows.len() != self.rows.len() {
            self.rows = snapshot.rows;
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let next = self.selected as i32 + delta;
        self.selected = next.clamp(0, self.rows.len() as i32 - 1) as usize;
    }

    pub async fn next_kind(&mut self) {
        self.kind_index = (self.kind_index + 1) % self.kinds.len();
        self.refresh().await;
        self.selected = 0;
    }

    pub async fn prev_kind(&mut self) {
        self.kind_index = if self.kind_index == 0 {
            self.kinds.len() - 1
        } else {
            self.kind_index - 1
        };
        self.refresh().await;
        self.selected = 0;
    }

    pub async fn next_namespace(&mut self) {
        if self.namespaces.is_empty() {
            return;
        }
        self.namespace_index = (self.namespace_index + 1) % self.namespaces.len();
        let ns = self.namespaces[self.namespace_index].clone();
        let _ = self.manager.set_namespace(ns, self.current_kind()).await;
        self.refresh().await;
    }

    pub async fn describe_selected(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match self
            .manager
            .resource_yaml(self.current_kind(), &row.name)
            .await
        {
            Ok(yaml) => self.detail = yaml,
            Err(err) => self.detail = err.user_message(),
        }
    }
}

pub fn draw(frame: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(45),
            Constraint::Percentage(55),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled("rusticlens-tui", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  q quit  r refresh  d describe  Tab kind  Ctrl+n namespace"),
    ]))
    .block(Block::default().borders(Borders::ALL).title(app.status.clone()));
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let style = if idx == app.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(format!(
                "{:<32} {:<8} {:<12} {}",
                row.name, row.ready, row.status, row.age
            ))
            .style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(app.current_kind().label()),
    );
    frame.render_widget(list, chunks[1]);

    let detail = Paragraph::new(app.detail.as_str()).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Describe"),
    );
    frame.render_widget(detail, chunks[2]);
}
