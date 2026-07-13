use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, Table,
    },
    Frame,
};

use crate::app::{ConnectionState, DetailTab, FocusPane, Overlay, TuiApp};
use rl_core::{config::context_is_usable, ResourceCategory, ResourceKind};

const ACCENT: Color = Color::Rgb(0, 191, 165);
const MUTED: Color = Color::Rgb(140, 147, 158);
const ERROR: Color = Color::Rgb(244, 67, 54);
const MATCH_HIGHLIGHT: Color = Color::Rgb(255, 235, 59);
const MATCH_ACTIVE: Color = Color::Rgb(0, 96, 100);
const YANK_HIGHLIGHT: Color = Color::Rgb(45, 55, 72);

pub fn draw(frame: &mut Frame, app: &mut TuiApp) {
    let area = frame.area();

    if app.log_view.is_some() {
        draw_log_view(frame, area, app);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);

    match &app.connection {
        ConnectionState::Disconnected | ConnectionState::Connecting => draw_center_message(
            frame,
            chunks[1],
            "Connecting to cluster...",
            Some("Ensure kubeconfig is valid. For Teleport, run `tsh login` first."),
        ),
        ConnectionState::Failed(msg) => {
            draw_center_message(frame, chunks[1], "Failed to connect", Some(msg))
        }
        ConnectionState::Connected => draw_main_panels(frame, chunks[1], app),
    }

    draw_footer(frame, chunks[2], app);

    if let Some(overlay) = &app.overlay {
        draw_overlay(frame, area, app, overlay);
    }
}

fn draw_header(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let (context, namespace, kind_label, count) = match &app.connection {
        ConnectionState::Connected => (
            app.manager.as_ref().map(|m| m.context().to_string()),
            app.manager.as_ref().map(|m| m.namespace().to_string()),
            Some(app.active_kind.label()),
            Some(app.rows.len()),
        ),
        ConnectionState::Disconnected | ConnectionState::Connecting => (None, None, None, None),
        ConnectionState::Failed(_) => (None, None, None, None),
    };

    let mut spans = vec![Span::styled(
        " rusticlens-tui ",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];

    if let Some(ctx) = context {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(ctx, Style::default().fg(Color::White)));
    }
    if let Some(ns) = namespace {
        spans.push(Span::raw(" / "));
        spans.push(Span::styled(ns, Style::default().fg(Color::White)));
    }
    if let Some(kind) = kind_label {
        spans.push(Span::raw(" — "));
        spans.push(Span::styled(kind, Style::default().fg(ACCENT)));
    }
    if let Some(n) = count {
        spans.push(Span::raw(format!(" ({n})")));
    }
    if let Some(err) = &app.error_message {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(err.clone(), Style::default().fg(ERROR)));
    } else if !app.status_message.is_empty() {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            app.status_message.clone(),
            Style::default().fg(MUTED),
        ));
    }

    let header = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    frame.render_widget(header, area);
}

fn draw_center_message(frame: &mut Frame, area: Rect, title: &str, subtitle: Option<&str>) {
    let mut lines = vec![Line::from(Span::styled(
        title,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    if let Some(sub) = subtitle {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(sub, Style::default().fg(MUTED))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press r to retry, q to quit",
        Style::default().fg(MUTED),
    )));

    let msg =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" rusticlens "));
    frame.render_widget(msg, area);
}

fn draw_main_panels(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(area);

    draw_sidebar(frame, chunks[0], app);
    draw_table(frame, chunks[1], app);
    draw_detail(frame, chunks[2], app);
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let mut lines: Vec<Line> = Vec::new();

    for category in [
        ResourceCategory::Workloads,
        ResourceCategory::Network,
        ResourceCategory::Config,
        ResourceCategory::Cluster,
        ResourceCategory::Custom,
    ] {
        lines.push(Line::from(Span::styled(
            category_title(category),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        )));

        for kind in kinds_in_category(category) {
            let active = app.active_kind == kind;
            let sidebar_selected =
                app.focus == FocusPane::Sidebar && app.sidebar_index == kind_sidebar_index(kind);

            let mut style = if active {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            if sidebar_selected {
                style = style.add_modifier(Modifier::REVERSED);
            }

            let prefix = if active { "▸ " } else { "  " };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}", kind.label()),
                style,
            )));
        }
        lines.push(Line::from(""));
    }

    // Helm releases (GUI has separate Helm section)
    lines.push(Line::from(Span::styled(
        "Helm",
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
    )));
    let helm_active = app.active_kind == ResourceKind::HelmRelease;
    let helm_selected = app.focus == FocusPane::Sidebar
        && app.sidebar_index == kind_sidebar_index(ResourceKind::HelmRelease);
    let mut helm_style = if helm_active {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    if helm_selected {
        helm_style = helm_style.add_modifier(Modifier::REVERSED);
    }
    let helm_prefix = if helm_active { "▸ " } else { "  " };
    lines.push(Line::from(Span::styled(
        format!("{helm_prefix}Helm Releases"),
        helm_style,
    )));

    if app.active_kind == ResourceKind::Crd && !app.crd_targets.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "CRD type",
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        )));
        for (idx, target) in app.crd_targets.iter().enumerate() {
            let selected = idx == app.selected_crd_index;
            let style = if selected {
                Style::default().fg(ACCENT).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(
                format!("  {}", target.display_name),
                style,
            )));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Navigation ")
        .border_style(if app.focus == FocusPane::Sidebar {
            Style::default().fg(ACCENT)
        } else {
            Style::default()
        });

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_table(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let kind = app.active_kind;
    let mut title = if kind == ResourceKind::Crd && app.crd_targets.is_empty() {
        "Custom Resources (no CRDs found)".to_string()
    } else if kind == ResourceKind::Crd {
        "Custom Resources".to_string()
    } else {
        kind.label().to_string()
    };

    if kind == ResourceKind::Pod && (!app.table_filter.is_empty() || app.search_mode) {
        let filter = if app.table_filter.is_empty() {
            "_".to_string()
        } else {
            app.table_filter.clone()
        };
        let marker = if app.search_mode { "▸" } else { " " };
        title = format!("{title} {marker} /{filter}");
    }

    let border_style = if app.focus == FocusPane::Table {
        Style::default().fg(ACCENT)
    } else {
        Style::default()
    };

    let indices = app.filtered_row_indices();

    if app.rows.is_empty() {
        let empty = Paragraph::new("No resources found. Press r to refresh.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(border_style),
        );
        frame.render_widget(empty, area);
        return;
    }

    if indices.is_empty() {
        let empty = Paragraph::new("No pods match the current filter.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} "))
                .border_style(border_style),
        );
        frame.render_widget(empty, area);
        return;
    }

    let (header, widths) = table_columns(kind);
    let header_row =
        Row::new(header).style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = indices
        .iter()
        .enumerate()
        .map(|(visible_idx, row_idx)| {
            let row = &app.rows[*row_idx];
            let style = if visible_idx == app.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Row::new(format_row(kind, row)).style(style)
        })
        .collect();

    let count_suffix = if kind == ResourceKind::Pod && !app.table_filter.is_empty() {
        format!(" ({}/{})", indices.len(), app.rows.len())
    } else {
        String::new()
    };

    let table = Table::new(rows, widths)
        .header(header_row)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title}{count_suffix} "))
                .border_style(border_style),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut table_state = ratatui::widgets::TableState::default();
    table_state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let border_style = if app.focus == FocusPane::Detail {
        Style::default().fg(ACCENT)
    } else {
        Style::default()
    };

    let tab_line = format!(
        "{}Describe{} | {}Events{} | {}Metrics{}",
        if app.detail_tab == DetailTab::Describe {
            "["
        } else {
            " "
        },
        if app.detail_tab == DetailTab::Describe {
            "]"
        } else {
            " "
        },
        if app.detail_tab == DetailTab::Events {
            "["
        } else {
            " "
        },
        if app.detail_tab == DetailTab::Events {
            "]"
        } else {
            " "
        },
        if app.detail_tab == DetailTab::Metrics {
            "["
        } else {
            " "
        },
        if app.detail_tab == DetailTab::Metrics {
            "]"
        } else {
            " "
        },
    );

    let content = match app.detail_tab {
        DetailTab::Describe => &app.detail_yaml,
        DetailTab::Events => &app.detail_events,
        DetailTab::Metrics => &app.detail_metrics,
    };

    let display = if content.is_empty() {
        "Select a resource and press d to load details.".to_string()
    } else {
        content.clone()
    };

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(area);

    let tabs = Paragraph::new(Line::from(Span::styled(
        tab_line,
        Style::default().fg(MUTED),
    )))
    .block(
        Block::default()
            .borders(Borders::LEFT | Borders::TOP | Borders::RIGHT)
            .border_style(border_style),
    );
    frame.render_widget(tabs, inner[0]);

    let lines: Vec<Line> = display.lines().map(|l| Line::from(l.to_string())).collect();
    let visible_height = inner[1].height.saturating_sub(2) as usize;
    let scroll = app.detail_scroll as usize;
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    let detail = Paragraph::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Detail ")
            .border_style(border_style),
    );
    frame.render_widget(detail, inner[1]);

    if scroll > 0 || display.lines().count() > visible_height + scroll {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(display.lines().count())
            .position(scroll);
        frame.render_stateful_widget(scrollbar, inner[1], &mut scroll_state);
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let help = if app.overlay_open() {
        "↑/↓ move | Enter switch | Esc cancel | type to filter"
    } else if app.search_mode {
        "type to filter pods | Enter done | Esc clear/close"
    } else {
        match &app.connection {
            ConnectionState::Connected => {
                "q quit | r refresh | / pod search | Tab kind | c context | n namespace | L logs | h/l focus | d detail"
            }
            ConnectionState::Disconnected | ConnectionState::Connecting => "q quit | connecting...",
            ConnectionState::Failed(_) => "q quit | r retry connect",
        }
    };

    let focus = match app.focus {
        FocusPane::Sidebar => "sidebar",
        FocusPane::Table => "table",
        FocusPane::Detail => "detail",
    };

    let text = if app.overlay_open() {
        help.to_string()
    } else if matches!(app.connection, ConnectionState::Connected) {
        format!("{help} | focus: {focus}")
    } else {
        help.to_string()
    };

    let footer = Paragraph::new(text).style(Style::default().fg(MUTED));
    frame.render_widget(footer, area);
}

fn draw_overlay(frame: &mut Frame, area: Rect, app: &TuiApp, overlay: &Overlay) {
    match overlay {
        Overlay::Context(state) => draw_context_picker(frame, area, app, state),
        Overlay::Namespace(state) => draw_namespace_picker(frame, area, app, state),
        Overlay::Container {
            pod_name,
            containers,
            state,
        } => draw_container_picker(frame, area, pod_name, containers, state),
    }
}

fn draw_context_picker(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    state: &crate::app::ListPickerState,
) {
    let active_context = app.manager.as_ref().map(|m| m.context());
    let indices = app.picker_indices();
    let list_height = centered_rect(70, 70, area).height.saturating_sub(6) as usize;

    let lines = build_picker_lines(
        app,
        &indices,
        state.selected,
        list_height,
        |app, idx| app.contexts[idx].as_str(),
        |name| {
            let usable = context_is_usable(name);
            let active = active_context == Some(name);
            let mut label = name.to_string();
            if active {
                label.push_str(" (current)");
            }
            if !usable {
                label.push_str(" (broken)");
            }
            let style = if active && usable {
                Style::default().fg(ACCENT)
            } else if usable {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(ERROR).add_modifier(Modifier::DIM)
            };
            (label, style)
        },
    );

    draw_searchable_popup(
        frame,
        area,
        " Switch context ",
        &state.search,
        &lines,
        "Contexts from kubeconfig — broken entries reference a missing cluster or user",
        if indices.is_empty() {
            "No matching contexts"
        } else {
            ""
        },
    );
}

fn draw_namespace_picker(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    state: &crate::app::ListPickerState,
) {
    let active_namespace = app.manager.as_ref().map(|m| m.namespace());
    let indices = app.picker_indices();
    let list_height = centered_rect(70, 70, area).height.saturating_sub(6) as usize;

    let lines = build_picker_lines(
        app,
        &indices,
        state.selected,
        list_height,
        |app, idx| app.namespaces[idx].as_str(),
        |name| {
            let active = active_namespace == Some(name);
            let label = if active {
                format!("{name} (current)")
            } else {
                name.to_string()
            };
            let style = if active {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(Color::White)
            };
            (label, style)
        },
    );

    draw_searchable_popup(
        frame,
        area,
        " Switch namespace ",
        &state.search,
        &lines,
        "Namespaces in the current cluster",
        if indices.is_empty() {
            "No matching namespaces"
        } else {
            ""
        },
    );
}

fn draw_container_picker(
    frame: &mut Frame,
    area: Rect,
    pod_name: &str,
    containers: &[String],
    state: &crate::app::ListPickerState,
) {
    let indices: Vec<usize> = {
        let query = state.search.to_lowercase();
        containers
            .iter()
            .enumerate()
            .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
            .map(|(idx, _)| idx)
            .collect()
    };
    let list_height = centered_rect(70, 70, area).height.saturating_sub(6) as usize;

    let lines = build_picker_lines_from_strings(
        &indices,
        state.selected,
        list_height,
        |idx| containers[idx].clone(),
        |name| (name.to_string(), Style::default().fg(Color::White)),
    );

    draw_searchable_popup(
        frame,
        area,
        &format!(" Logs — {pod_name} "),
        &state.search,
        &lines,
        "Select a container to stream logs from",
        if indices.is_empty() {
            "No matching containers"
        } else {
            ""
        },
    );
}

fn draw_log_view(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let show_search = app
        .log_view
        .as_ref()
        .is_some_and(|log| log.search_mode || !log.search_query.is_empty());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(if show_search { 2 } else { 0 }),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);

    let body_area = chunks[2];
    let footer_area = chunks[3];
    let body_height = body_area.height as usize;

    let Some(log) = app.log_view.as_ref() else {
        return;
    };

    let container = log.container.as_deref().unwrap_or("default");
    let namespace = app.manager.as_ref().map(|m| m.namespace()).unwrap_or("?");
    let follow_label = if log.follow { "on" } else { "off" };
    let match_label = if log.search_query.is_empty() {
        String::new()
    } else if log.match_rows.is_empty() {
        " | no matches".to_string()
    } else {
        format!(" | match {}/{}", log.match_cursor + 1, log.match_rows.len())
    };

    let title = format!(
        " {} / {} — container: {} | {} lines | follow: {follow_label}{match_label} ",
        namespace,
        log.pod_name,
        container,
        log.lines.len()
    );

    let mut header_spans = vec![Span::styled(
        title,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if let Some(err) = &log.error {
        header_spans.push(Span::raw(" | "));
        header_spans.push(Span::styled(err.clone(), Style::default().fg(ERROR)));
    }

    let header = Paragraph::new(Line::from(header_spans))
        .block(Block::default().borders(Borders::ALL).title(" Pod logs "));
    frame.render_widget(header, chunks[0]);

    if show_search {
        let query = if log.search_query.is_empty() {
            "_".to_string()
        } else {
            log.search_query.clone()
        };
        let marker = if log.search_mode { "▸" } else { " " };
        let search_bar = Paragraph::new(Line::from(vec![
            Span::styled(format!("{marker} /"), Style::default().fg(ACCENT)),
            Span::raw(" "),
            Span::styled(query, Style::default().fg(Color::White)),
        ]));
        frame.render_widget(search_bar, chunks[1]);
    }

    app.prepare_log_view(body_height, body_area.width.max(1) as usize);
    let show_scrollbar = app
        .log_view
        .as_ref()
        .is_some_and(|log| log.wrapped_lines().len() > body_height);
    let content_width = if show_scrollbar {
        body_area.width.saturating_sub(1).max(1) as usize
    } else {
        body_area.width.max(1) as usize
    };
    if app
        .log_view
        .as_ref()
        .is_some_and(|log| log.wrap_width != content_width)
    {
        app.prepare_log_view(body_height, content_width);
    }

    let Some(log) = app.log_view.as_ref() else {
        return;
    };
    let wrapped = log.wrapped_lines();
    let visible_height = body_height;
    let scroll = log.scroll.min(wrapped.len().saturating_sub(visible_height));
    let current_match_row = log.current_match_row();
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(if show_scrollbar { 1 } else { 0 }),
        ])
        .split(body_area);
    let log_area = log_chunks[0];
    app.log_layout = Some(crate::app::LogViewLayout {
        area: log_area,
        scroll,
    });

    let highlight_source = log.highlight_source;
    let visible: Vec<Line> = if wrapped.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for log output…",
            Style::default().fg(MUTED),
        ))]
    } else {
        wrapped
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible_height)
            .map(|(row_idx, line)| {
                let is_active = current_match_row == Some(row_idx);
                let is_yanked = highlight_source
                    .is_some_and(|src| log.source_line_for_wrapped_row(row_idx) == src);
                highlight_log_line(line, &log.search_query, is_active, is_yanked)
            })
            .collect()
    };

    // No side borders — box-drawing chars break terminal mouse line selection.
    let body = Paragraph::new(visible);
    frame.render_widget(body, log_area);

    if show_scrollbar {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(wrapped.len())
            .position(scroll);
        frame.render_stateful_widget(scrollbar, log_chunks[1], &mut scroll_state);
    }

    let footer_text = if log.search_mode {
        "type search | Enter find | Esc clear/close search"
    } else {
        "Esc/q close | / search | n/N match | ↑/↓ scroll | f follow | g/G top/bottom | double-click/`y` copy line"
    };
    let footer = Paragraph::new(footer_text).style(Style::default().fg(MUTED));
    frame.render_widget(footer, footer_area);
}

fn highlight_log_line(
    line: &str,
    query: &str,
    is_active_row: bool,
    is_yanked_row: bool,
) -> Line<'static> {
    if is_yanked_row {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White).bg(YANK_HIGHLIGHT),
        ));
    }

    if query.is_empty() {
        let style = if is_active_row {
            Style::default()
                .fg(Color::White)
                .bg(MATCH_ACTIVE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        return Line::from(Span::styled(line.to_string(), style));
    }

    let q_lower = query.to_lowercase();
    let mut spans: Vec<Span> = Vec::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        let rem_lower = remaining.to_lowercase();
        let Some(pos) = rem_lower.find(&q_lower) else {
            spans.push(Span::raw(remaining.to_string()));
            break;
        };
        if pos > 0 {
            spans.push(Span::raw(remaining[..pos].to_string()));
        }
        let match_len = query.len().min(remaining.len().saturating_sub(pos));
        let matched = remaining[pos..pos + match_len].to_string();
        let mut hit_style = Style::default().fg(Color::Black).bg(MATCH_HIGHLIGHT);
        if is_active_row {
            hit_style = hit_style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(matched, hit_style));
        remaining = &remaining[pos + match_len..];
    }

    if is_active_row {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::White)
                .bg(MATCH_ACTIVE)
                .add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(spans)
}

struct PickerLine {
    label: String,
    style: Style,
    selected: bool,
}

fn build_picker_lines(
    app: &TuiApp,
    indices: &[usize],
    selected: usize,
    list_height: usize,
    name_at: impl Fn(&TuiApp, usize) -> &str,
    decorate: impl Fn(&str) -> (String, Style),
) -> Vec<PickerLine> {
    let scroll = selected.saturating_sub(list_height.saturating_sub(1));
    let mut lines = Vec::new();

    for (idx, &item_idx) in indices.iter().enumerate().skip(scroll) {
        if lines.len() >= list_height {
            break;
        }
        let name = name_at(app, item_idx);
        let (label, style) = decorate(name);
        lines.push(PickerLine {
            label,
            style,
            selected: idx == selected,
        });
    }

    lines
}

fn build_picker_lines_from_strings(
    indices: &[usize],
    selected: usize,
    list_height: usize,
    name_at: impl Fn(usize) -> String,
    decorate: impl Fn(&str) -> (String, Style),
) -> Vec<PickerLine> {
    let scroll = selected.saturating_sub(list_height.saturating_sub(1));
    let mut lines = Vec::new();

    for (idx, &item_idx) in indices.iter().enumerate().skip(scroll) {
        if lines.len() >= list_height {
            break;
        }
        let name = name_at(item_idx);
        let (label, style) = decorate(&name);
        lines.push(PickerLine {
            label,
            style,
            selected: idx == selected,
        });
    }

    lines
}

fn draw_searchable_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    search: &str,
    lines: &[PickerLine],
    hint: &str,
    empty_message: &str,
) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(popup);

    let filter = Paragraph::new(Line::from(vec![
        Span::styled("Filter: ", Style::default().fg(MUTED)),
        Span::styled(
            if search.is_empty() {
                "_".to_string()
            } else {
                search.to_string()
            },
            Style::default().fg(Color::White),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(ACCENT)),
    );
    frame.render_widget(filter, chunks[0]);

    let rendered = if !empty_message.is_empty() {
        vec![Line::from(Span::styled(
            empty_message,
            Style::default().fg(MUTED),
        ))]
    } else {
        lines
            .iter()
            .map(|line| {
                let marker = if line.selected { "▸ " } else { "  " };
                let style = if line.selected {
                    line.style.add_modifier(Modifier::REVERSED)
                } else {
                    line.style
                };
                Line::from(Span::styled(format!("{marker}{}", line.label), style))
            })
            .collect()
    };

    let list = Paragraph::new(rendered).block(
        Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(ACCENT)),
    );
    frame.render_widget(list, chunks[1]);

    let footer = Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(MUTED)))).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT)),
    );
    frame.render_widget(footer, chunks[2]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn category_title(category: ResourceCategory) -> &'static str {
    match category {
        ResourceCategory::Workloads => "Workloads",
        ResourceCategory::Network => "Network",
        ResourceCategory::Storage => "Storage",
        ResourceCategory::Access => "Access",
        ResourceCategory::Config => "Config",
        ResourceCategory::Cluster => "Cluster",
        ResourceCategory::Custom => "Custom",
    }
}

fn kinds_in_category(category: ResourceCategory) -> Vec<ResourceKind> {
    ResourceKind::ALL
        .into_iter()
        .filter(|k| k.category() == category && *k != ResourceKind::HelmRelease)
        .collect()
}

pub fn kind_sidebar_index(kind: ResourceKind) -> usize {
    ResourceKind::ALL
        .iter()
        .position(|k| *k == kind)
        .unwrap_or(0)
}

fn table_columns(kind: ResourceKind) -> (Vec<Cell<'static>>, Vec<Constraint>) {
    match kind {
        ResourceKind::Pod => (
            vec![
                Cell::from("Name"),
                Cell::from("Ready"),
                Cell::from("Status"),
                Cell::from("Restarts"),
                Cell::from("Age"),
            ],
            vec![
                Constraint::Percentage(35),
                Constraint::Percentage(12),
                Constraint::Percentage(23),
                Constraint::Percentage(12),
                Constraint::Percentage(18),
            ],
        ),
        ResourceKind::Deployment => (
            vec![
                Cell::from("Name"),
                Cell::from("Ready"),
                Cell::from("Up-to-date"),
                Cell::from("Available"),
                Cell::from("Age"),
            ],
            vec![
                Constraint::Percentage(30),
                Constraint::Percentage(15),
                Constraint::Percentage(18),
                Constraint::Percentage(17),
                Constraint::Percentage(20),
            ],
        ),
        ResourceKind::CronJob => (
            vec![
                Cell::from("Name"),
                Cell::from("Schedule"),
                Cell::from("Suspended"),
                Cell::from("Active"),
                Cell::from("Age"),
            ],
            vec![
                Constraint::Percentage(28),
                Constraint::Percentage(22),
                Constraint::Percentage(15),
                Constraint::Percentage(15),
                Constraint::Percentage(20),
            ],
        ),
        _ => (
            vec![
                Cell::from("Name"),
                Cell::from("Ready"),
                Cell::from("Status"),
                Cell::from("Age"),
            ],
            vec![
                Constraint::Percentage(40),
                Constraint::Percentage(15),
                Constraint::Percentage(25),
                Constraint::Percentage(20),
            ],
        ),
    }
}

fn format_row(kind: ResourceKind, row: &rl_core::ResourceRow) -> Vec<Cell<'static>> {
    match kind {
        ResourceKind::Pod => vec![
            Cell::from(truncate(&row.name, 28)),
            Cell::from(row.ready.clone()),
            Cell::from(truncate(&row.status, 16)),
            Cell::from(row.restarts.clone()),
            Cell::from(row.age.clone()),
        ],
        ResourceKind::Deployment => vec![
            Cell::from(truncate(&row.name, 24)),
            Cell::from(row.ready.clone()),
            Cell::from(row.up_to_date.clone()),
            Cell::from(row.active.clone()),
            Cell::from(row.age.clone()),
        ],
        ResourceKind::CronJob => vec![
            Cell::from(truncate(&row.name, 22)),
            Cell::from(truncate(&row.schedule, 14)),
            Cell::from(row.resumed.clone()),
            Cell::from(row.active.clone()),
            Cell::from(row.age.clone()),
        ],
        _ => vec![
            Cell::from(truncate(&row.name, 32)),
            Cell::from(row.ready.clone()),
            Cell::from(truncate(&row.status, 18)),
            Cell::from(row.age.clone()),
        ],
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}
