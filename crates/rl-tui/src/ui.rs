use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation},
    Frame,
};

use crate::app::{
    ConnectionState, DetailTab, FocusPane, Overlay, SettingsCursor, TuiApp, ViewMode,
};
use crate::theme::ThemeColors;
use rl_core::{config::context_is_usable, ResourceCategory, ResourceKind};

fn colors(app: &TuiApp) -> ThemeColors {
    ThemeColors::for_mode(app.theme_mode)
}

// Base palette (dark). Theme toggle applies colors(app) in header/footer/overlays/overview.
const ACCENT: Color = Color::Rgb(0, 191, 165);
const MUTED: Color = Color::Rgb(140, 147, 158);
const ERROR: Color = Color::Rgb(244, 67, 54);
const MATCH_HIGHLIGHT: Color = Color::Rgb(255, 235, 59);
const MATCH_ACTIVE: Color = Color::Rgb(0, 96, 100);
const YANK_HIGHLIGHT: Color = Color::Rgb(45, 55, 72);

pub fn draw(frame: &mut Frame, app: &mut TuiApp) {
    let area = frame.area();
    // Wipe the whole frame first so shrinks / view switches cannot leave ghost panels.
    frame.render_widget(Clear, area);

    if app.log_view.is_some() {
        draw_log_view(frame, area, app);
        capture_and_paint_selection(frame, app);
        return;
    }

    let tab_h = if app.cluster_tabs.len() > 1 { 1 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(tab_h),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    if tab_h > 0 {
        draw_cluster_tabs(frame, chunks[1], app);
    }

    let main = chunks[2];
    let footer = chunks[3];

    match &app.connection {
        ConnectionState::Disconnected | ConnectionState::Connecting => draw_center_message(
            frame,
            main,
            "Connecting to cluster...",
            Some("Ensure kubeconfig is valid. For Teleport, run `tsh login` first."),
            app,
        ),
        ConnectionState::Failed(msg) => {
            let msg = msg.clone();
            draw_center_message(frame, main, "Failed to connect", Some(&msg), app)
        }
        ConnectionState::Connected if app.view_mode == ViewMode::Overview => {
            app.detail_layout = None;
            app.table_layout = None;
            draw_overview(frame, main, app)
        }
        ConnectionState::Connected => draw_main_panels(frame, main, app),
    }

    draw_footer(frame, footer, app);

    if let Some(overlay) = app.overlay.clone() {
        draw_overlay(frame, area, app, &overlay);
    }

    capture_and_paint_selection(frame, app);
}

/// Snapshot the rendered frame and overlay a drag-selection highlight so any
/// on-screen text (header, sidebar, table, detail, overlays, logs) can be copied.
pub fn capture_and_paint_selection(frame: &mut Frame, app: &mut TuiApp) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    let mut cells = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut row = Vec::with_capacity(area.width as usize);
        for x in area.left()..area.right() {
            let symbol = buf
                .cell((x, y))
                .map(|c| c.symbol().to_string())
                .unwrap_or_default();
            row.push(symbol);
        }
        cells.push(row);
    }
    app.screen_cells = cells;

    let Some(sel) = app.screen_selection else {
        return;
    };
    let ((sy, sx), (ey, ex)) = sel.normalized();
    let c = colors(app);
    let style = Style::default()
        .fg(Color::Black)
        .bg(c.match_highlight)
        .add_modifier(Modifier::BOLD);
    let max_y = area.height.saturating_sub(1) as usize;
    let max_x = area.width.saturating_sub(1) as usize;
    let ey = ey.min(max_y);
    let sy = sy.min(max_y);
    for y in sy..=ey {
        let from = if y == sy { sx.min(max_x) } else { 0 };
        let to = if y == ey { ex.min(max_x) } else { max_x };
        if from > to {
            continue;
        }
        for x in from..=to {
            let px = x as u16 + area.left();
            let py = y as u16 + area.top();
            if let Some(cell) = buf.cell_mut((px, py)) {
                cell.set_style(style);
            }
        }
    }
}

fn draw_cluster_tabs(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let c = colors(app);
    let active = if app.active_context.is_empty() {
        None
    } else {
        Some(app.active_context.as_str())
    };
    let mut spans = Vec::new();
    for tab in &app.cluster_tabs {
        let selected = active == Some(tab.as_str());
        spans.push(Span::styled(
            format!(" {tab} "),
            if selected {
                Style::default()
                    .fg(c.accent)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(c.muted)
            },
        ));
        spans.push(Span::styled("│", Style::default().fg(c.muted)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let c = colors(app);
    let (context, namespace, kind_label, count) = match &app.connection {
        ConnectionState::Connected => (
            if app.active_context.is_empty() {
                None
            } else {
                Some(app.active_context.clone())
            },
            if app.active_namespace.is_empty() {
                None
            } else {
                Some(app.active_namespace.clone())
            },
            Some(app.active_kind.label()),
            Some(app.rows.len()),
        ),
        ConnectionState::Disconnected | ConnectionState::Connecting => (None, None, None, None),
        ConnectionState::Failed(_) => (None, None, None, None),
    };

    let mut spans = vec![Span::styled(
        " rusticlens-tui ",
        Style::default().fg(c.accent).add_modifier(Modifier::BOLD),
    )];

    if let Some(ctx) = context {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(ctx, Style::default().fg(c.text)));
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
        spans.push(Span::styled(err.clone(), Style::default().fg(c.error)));
    } else if !app.status_message.is_empty() {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            app.status_message.clone(),
            Style::default().fg(c.muted),
        ));
    }

    let header = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    frame.render_widget(header, area);
}

fn draw_center_message(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    subtitle: Option<&str>,
    app: &TuiApp,
) {
    let c = colors(app);
    let mut lines = vec![Line::from(Span::styled(
        title,
        Style::default().fg(c.text).add_modifier(Modifier::BOLD),
    ))];
    if let Some(sub) = subtitle {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(sub, Style::default().fg(c.muted))));
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

fn draw_main_panels(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let show_detail = app.detail_panel_visible();
    let chunks = if show_detail {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Percentage(55),
                Constraint::Percentage(45),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(1)])
            .split(area)
    };

    draw_sidebar(frame, chunks[0], app);
    draw_table(frame, chunks[1], app);
    if show_detail && chunks.len() > 2 {
        draw_detail(frame, chunks[2], app);
    } else {
        app.detail_layout = None;
    }
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let mut lines: Vec<Line> = Vec::new();

    for category in [
        ResourceCategory::Workloads,
        ResourceCategory::Network,
        ResourceCategory::Storage,
        ResourceCategory::Access,
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

fn draw_table(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let c = colors(app);
    let kind = app.active_kind;
    let mut title = if kind == ResourceKind::Crd && app.crd_targets.is_empty() {
        "Custom Resources (no CRDs found)".to_string()
    } else if kind == ResourceKind::Crd {
        "Custom Resources".to_string()
    } else {
        kind.label().to_string()
    };

    if !app.table_filter.is_empty() || app.search_mode {
        let filter = if app.table_filter.is_empty() {
            "_".to_string()
        } else {
            app.table_filter.clone()
        };
        let marker = if app.search_mode { "▸" } else { " " };
        title = format!("{title} {marker} /{filter}");
    }

    let border_style = if app.focus == FocusPane::Table {
        Style::default().fg(c.accent)
    } else {
        Style::default()
    };

    let indices = app.filtered_row_indices();
    let count_suffix = if !app.table_filter.is_empty() {
        format!(" ({}/{})", indices.len(), app.rows.len())
    } else {
        String::new()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title}{count_suffix} "))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let header_area = chunks[0];
    let body = chunks[1];
    let visible_height = body.height.max(1) as usize;
    let name_width = name_column_width(kind, body.width as usize);

    app.table_lines = indices
        .iter()
        .map(|row_idx| format_row_line(kind, &app.rows[*row_idx], name_width))
        .collect();

    // Clamp scroll for current list size — never snap back to the selected row here.
    let max_scroll = app.table_lines.len().saturating_sub(visible_height);
    app.table_scroll = app.table_scroll.min(max_scroll);
    app.table_layout = Some(crate::app::TableLayout {
        panel: area,
        area: body,
        scroll: app.table_scroll,
    });

    if app.table_lines.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.rows.is_empty() {
                "No resources found. Press r to refresh."
            } else {
                "No resources match the current filter."
            })
            .style(Style::default().fg(c.muted)),
            inner,
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            table_header_line(kind, name_width),
            Style::default().fg(c.muted).add_modifier(Modifier::BOLD),
        ))),
        header_area,
    );

    let scroll = app.table_scroll;
    let selection = app.table_selection;
    let lines: Vec<Line> = app
        .table_lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(line_idx, text)| {
            let mut line = highlight_detail_line(
                text,
                line_idx,
                selection,
                "",
                false,
                0,
                text.chars().count().max(1),
                c,
            );
            if line_idx == app.selected && selection.is_none() {
                line = Line::from(Span::styled(
                    text.clone(),
                    Style::default().fg(c.text).add_modifier(Modifier::REVERSED),
                ));
            }
            line
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), body);

    if app.table_lines.len() > visible_height {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        // content_length = scroll positions (0..=max_scroll); viewport=1 keeps the thumb
        // small while still letting position=max_scroll land at the track end.
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(max_scroll.saturating_add(1))
            .viewport_content_length(1)
            .position(scroll);
        frame.render_stateful_widget(scrollbar, body, &mut scroll_state);
    }
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &mut TuiApp) {
    let c = colors(app);
    let border_style = if app.focus == FocusPane::Detail {
        Style::default().fg(c.accent)
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

    let content = app.detail_content();
    let display = if app.detail_loading() && content.is_empty() {
        "Loading…".to_string()
    } else if content.is_empty() {
        "Select a resource and press d to load details.".to_string()
    } else {
        content.to_string()
    };
    let all_lines: Vec<&str> = display.lines().collect();
    let max_line_chars = all_lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(area);

    let tabs = Paragraph::new(Line::from(Span::styled(
        tab_line,
        Style::default().fg(c.muted),
    )))
    .block(
        Block::default()
            .borders(Borders::LEFT | Borders::TOP | Borders::RIGHT)
            .border_style(border_style),
    );
    frame.render_widget(tabs, inner[0]);

    let show_search = app.detail_search_visible();
    let approx_width = inner[1].width.saturating_sub(2).max(1) as usize;
    let approx_max_x = max_line_chars.saturating_sub(approx_width);
    let title = if approx_max_x > 0 {
        format!(
            " Detail · pan {}/{} (./← →/.) ",
            app.detail_scroll_x.min(approx_max_x),
            approx_max_x
        )
    } else {
        " Detail ".to_string()
    };

    let body_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style);
    let body_inner = body_block.inner(inner[1]);
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if show_search { 1 } else { 0 }),
            Constraint::Min(1),
        ])
        .split(body_inner);
    let content_area = body_chunks[1];
    let show_hscroll = max_line_chars > content_area.width.max(1) as usize;
    let text_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if show_hscroll { 1 } else { 0 }),
        ])
        .split(content_area);
    let text_area = text_chunks[0];
    let show_vscroll_hint = all_lines.len() > text_area.height.max(1) as usize;
    let text_width = if show_vscroll_hint {
        text_area.width.saturating_sub(1).max(1)
    } else {
        text_area.width.max(1)
    } as usize;
    let visible_height = text_area.height.max(1) as usize;
    let max_scroll = all_lines.len().saturating_sub(visible_height);
    let scroll = (app.detail_scroll as usize).min(max_scroll);
    app.detail_scroll = scroll as u16;
    let max_scroll_x = max_line_chars.saturating_sub(text_width);
    let scroll_x = app.detail_scroll_x.min(max_scroll_x);
    app.detail_scroll_x = scroll_x;
    app.detail_layout = Some(crate::app::DetailLayout {
        panel: area,
        area: text_area,
        hscroll_area: if show_hscroll {
            Some(text_chunks[1])
        } else {
            None
        },
        text_width,
        scroll,
        scroll_x,
    });

    frame.render_widget(body_block, inner[1]);

    if show_search {
        let query = if app.detail_search_query.is_empty() {
            String::new()
        } else {
            app.detail_search_query.clone()
        };
        let marker = if app.detail_search_mode { "▸" } else { " " };
        let match_label = if app.detail_search_query.is_empty() {
            String::new()
        } else if app.detail_match_rows.is_empty() {
            " · no matches".into()
        } else {
            format!(
                " · {}/{}",
                app.detail_match_cursor + 1,
                app.detail_match_rows.len()
            )
        };
        let search_bar = Paragraph::new(Line::from(vec![
            Span::styled(format!("{marker} /"), Style::default().fg(ACCENT)),
            Span::raw(" "),
            Span::styled(query, Style::default().fg(Color::White)),
            Span::styled(match_label, Style::default().fg(MUTED)),
        ]));
        frame.render_widget(search_bar, body_chunks[0]);
    }

    let selection = app.detail_selection;
    let query = app.detail_search_query.clone();
    let current_match = app.detail_current_match_row();
    // Full lines + Paragraph::scroll(y, x) so ratatui owns the horizontal viewport.
    let lines: Vec<Line> = all_lines
        .iter()
        .enumerate()
        .map(|(line_idx, text)| {
            let width = text.chars().count().max(1);
            highlight_detail_line(
                text,
                line_idx,
                selection,
                &query,
                current_match == Some(line_idx),
                0,
                width,
                c,
            )
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).scroll((scroll as u16, scroll_x as u16)),
        text_area,
    );

    if show_vscroll_hint || scroll > 0 {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(max_scroll.saturating_add(1))
            .viewport_content_length(1)
            .position(scroll.min(max_scroll));
        frame.render_stateful_widget(scrollbar, text_area, &mut scroll_state);
    }

    if show_hscroll {
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::HorizontalBottom)
            .begin_symbol(Some("←"))
            .end_symbol(Some("→"));
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(max_scroll_x.saturating_add(1))
            .viewport_content_length(1)
            .position(scroll_x.min(max_scroll_x));
        frame.render_stateful_widget(scrollbar, text_chunks[1], &mut scroll_state);
    }
}

#[allow(clippy::too_many_arguments)]
fn highlight_detail_line(
    text: &str,
    line_idx: usize,
    selection: Option<crate::app::DetailSelection>,
    query: &str,
    is_active_match: bool,
    scroll_x: usize,
    width: usize,
    c: crate::theme::ThemeColors,
) -> Line<'static> {
    let chars: Vec<char> = text.chars().collect();
    let end = (scroll_x + width).min(chars.len());
    let start = scroll_x.min(chars.len());
    let window: String = chars[start..end].iter().collect();
    let window_len = end.saturating_sub(start);

    if let Some(sel) = selection {
        let ((sl, sc), (el, ec)) = sel.normalized();
        if line_idx >= sl && line_idx <= el {
            let abs_from = if line_idx == sl {
                sc.min(chars.len())
            } else {
                0
            };
            let abs_to = if line_idx == el {
                ec.min(chars.len()).max(abs_from)
            } else {
                chars.len()
            };
            let from = abs_from.saturating_sub(scroll_x).min(window_len);
            let to = abs_to.saturating_sub(scroll_x).min(window_len);
            if from < to {
                let win_chars: Vec<char> = window.chars().collect();
                let mut spans = Vec::new();
                if from > 0 {
                    spans.push(Span::styled(
                        win_chars[..from].iter().collect::<String>(),
                        Style::default().fg(c.text),
                    ));
                }
                spans.push(Span::styled(
                    win_chars[from..to].iter().collect::<String>(),
                    Style::default()
                        .fg(Color::Black)
                        .bg(c.match_highlight)
                        .add_modifier(Modifier::BOLD),
                ));
                if to < win_chars.len() {
                    spans.push(Span::styled(
                        win_chars[to..].iter().collect::<String>(),
                        Style::default().fg(c.text),
                    ));
                }
                return Line::from(spans);
            }
        }
    }

    if is_active_match {
        return Line::from(Span::styled(
            window,
            Style::default()
                .fg(Color::White)
                .bg(MATCH_ACTIVE)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if query.is_empty() {
        return Line::from(Span::styled(window, Style::default().fg(c.text)));
    }

    // Highlight query hits that intersect the visible window (search absolute, paint relative).
    let q_lower = query.to_lowercase();
    let full_lower = text.to_lowercase();
    let mut spans: Vec<Span> = Vec::new();
    let mut painted = 0usize;
    let mut search_from = 0usize;
    while search_from < full_lower.len() {
        let Some(rel) = full_lower[search_from..].find(&q_lower) else {
            break;
        };
        let byte_start = search_from + rel;
        let byte_end = byte_start + query.len().min(full_lower.len().saturating_sub(byte_start));
        let abs_from = text
            .get(..byte_start)
            .map(|p| p.chars().count())
            .unwrap_or(0);
        let abs_to = text
            .get(..byte_end)
            .map(|p| p.chars().count())
            .unwrap_or(abs_from);
        let vis_from = abs_from.saturating_sub(scroll_x);
        let vis_to = abs_to.saturating_sub(scroll_x);
        if vis_to > painted && vis_from < window_len {
            let from = vis_from.max(painted).min(window_len);
            let to = vis_to.min(window_len);
            if from > painted {
                let slice: String = window.chars().skip(painted).take(from - painted).collect();
                spans.push(Span::styled(slice, Style::default().fg(c.text)));
            }
            if to > from {
                let slice: String = window.chars().skip(from).take(to - from).collect();
                spans.push(Span::styled(
                    slice,
                    Style::default().fg(Color::Black).bg(MATCH_HIGHLIGHT),
                ));
            }
            painted = to.max(painted);
        }
        search_from = byte_end.max(search_from + 1);
        if abs_to <= scroll_x {
            continue;
        }
        if abs_from >= scroll_x + width {
            break;
        }
    }
    if painted < window_len {
        let slice: String = window.chars().skip(painted).collect();
        spans.push(Span::styled(slice, Style::default().fg(c.text)));
    }
    if spans.is_empty() {
        Line::from(Span::styled(window, Style::default().fg(c.text)))
    } else {
        Line::from(spans)
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let c = colors(app);
    let help = if app.overlay_open() {
        match &app.overlay {
            Some(Overlay::Confirm { .. }) => "Enter confirm | Esc cancel",
            Some(Overlay::Input { .. }) => "type value | Enter confirm | Esc cancel",
            Some(Overlay::Help) => "Esc/Enter close | ? open anytime",
            Some(Overlay::Settings { .. }) => {
                "↑/↓ move | Enter toggle/add | d delete path | Esc close"
            }
            Some(Overlay::PortForwardList { .. }) => "↑/↓ | Enter stop | Esc close",
            _ => "↑/↓ move | Enter select | Esc cancel | type to filter",
        }
    } else if app.detail_search_mode {
        "type to search detail | Enter done | Esc clear/close | then n/N matches"
    } else if app.search_mode {
        "type to filter by name | Enter done | Esc clear/close"
    } else if app.view_mode == ViewMode::Overview {
        "o close overview | r refresh | ? help | q quit"
    } else {
        match &app.connection {
            ConnectionState::Connected => {
                if app.focus == FocusPane::Detail {
                    "q quit | Esc close | wheel=vert · Shift/Alt/Ctrl+wheel=pan · ←/→ | / find"
                } else if app.detail_panel_visible() {
                    "q quit | / filter | Esc close detail | Shift+wheel pan | d reload | ? help"
                } else {
                    "q quit | / filter | d describe | ? help | m actions"
                }
            }
            ConnectionState::Disconnected | ConnectionState::Connecting => {
                "q quit (abort connect) | waiting for kube..."
            }
            ConnectionState::Failed(_) => "q quit | r retry connect",
        }
    };

    let focus = match app.focus {
        FocusPane::Sidebar => "sidebar",
        FocusPane::Table => "table",
        FocusPane::Detail => "detail",
    };

    let mut text = if app.overlay_open() {
        help.to_string()
    } else if matches!(app.connection, ConnectionState::Connected) {
        format!("{help} | focus: {focus}")
    } else {
        help.to_string()
    };
    if !app.port_forward_entries.is_empty() {
        text.push_str(&format!(" | PF:{}", app.port_forward_entries.len()));
    }

    let footer = Paragraph::new(text).style(Style::default().fg(c.muted));
    frame.render_widget(footer, area);
}

fn draw_overview(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let c = colors(app);
    let mut lines = vec![
        Line::from(Span::styled(
            "Cluster Overview",
            Style::default().fg(c.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if !app.active_context.is_empty() {
        lines.push(Line::from(format!("Context: {}", app.active_context)));
        lines.push(Line::from(format!("Namespace: {}", app.active_namespace)));
        lines.push(Line::from(""));
    }
    match &app.dashboard {
        Some(d) => {
            lines.push(Line::from(format!(
                "Pods: total={} running={} pending={} failed={} succeeded={} unknown={}",
                d.total_pods,
                d.running_pods,
                d.pending_pods,
                d.failed_pods,
                d.succeeded_pods,
                d.unknown_pods
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Nodes",
                Style::default().fg(c.accent).add_modifier(Modifier::BOLD),
            )));
            for node in &d.nodes {
                lines.push(Line::from(format!(
                    "  {} ready={} pods={} cpu={} mem={}",
                    node.name, node.ready, node.pods, node.cpu_allocatable, node.memory_allocatable
                )));
            }
        }
        None => lines.push(Line::from(Span::styled(
            "Loading dashboard… (press r to refresh)",
            Style::default().fg(c.muted),
        ))),
    }
    let widget = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Overview ")
            .border_style(Style::default().fg(c.accent)),
    );
    frame.render_widget(widget, area);
}

fn draw_overlay(frame: &mut Frame, area: Rect, app: &TuiApp, overlay: &Overlay) {
    match overlay {
        Overlay::Context(state) => draw_context_picker(frame, area, app, state),
        Overlay::Namespace(state) => draw_namespace_picker(frame, area, app, state),
        Overlay::Container {
            pod_name,
            containers,
            state,
            purpose,
        } => {
            let title = match purpose {
                crate::app::ContainerPickerPurpose::Logs => " Select container (logs) ",
                crate::app::ContainerPickerPurpose::ExternalLogs => {
                    " Select container (logs → new terminal) "
                }
                crate::app::ContainerPickerPurpose::Exec => " Select container (exec) ",
            };
            draw_container_picker(frame, area, pod_name, containers, state, title)
        }
        Overlay::Confirm { title, detail, .. } => draw_simple_popup(
            frame,
            area,
            title,
            detail,
            "Enter confirm · Esc cancel",
            app,
        ),
        Overlay::Input { prompt, value, .. } => {
            let body = format!("{prompt}\n\n> {value}_");
            draw_simple_popup(
                frame,
                area,
                " Input ",
                &body,
                "Enter confirm · Esc cancel",
                app,
            )
        }
        Overlay::ActionMenu {
            items,
            selected,
            filter,
        } => draw_action_menu(frame, area, app, items, *selected, filter),
        Overlay::Favorites(state) => draw_favorites_picker(frame, area, app, state),
        Overlay::EditorPicker(state) => draw_editor_picker(frame, area, app, state),
        Overlay::PortForwardList { selected } => {
            draw_port_forward_list(frame, area, app, *selected)
        }
        Overlay::Settings {
            cursor,
            path_selected,
        } => draw_settings(frame, area, app, *cursor, *path_selected),
        Overlay::Help => draw_help(frame, area, app),
    }
}

fn draw_simple_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    body: &str,
    hint: &str,
    app: &TuiApp,
) {
    let c = colors(app);
    let popup = centered_rect(60, 40, area);
    frame.render_widget(Clear, popup);
    let lines: Vec<Line> = body.lines().map(|l| Line::from(l.to_string())).collect();
    let mut content = lines;
    content.push(Line::from(""));
    content.push(Line::from(Span::styled(hint, Style::default().fg(c.muted))));
    frame.render_widget(
        Paragraph::new(content).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title.to_string())
                .border_style(Style::default().fg(c.accent)),
        ),
        popup,
    );
}

fn draw_action_menu(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    items: &[crate::actions::ActionItem],
    selected: usize,
    filter: &str,
) {
    let c = colors(app);
    let filtered: Vec<(usize, &crate::actions::ActionItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            filter.is_empty() || item.label.to_lowercase().contains(&filter.to_lowercase())
        })
        .collect();
    let lines: Vec<PickerLine> = filtered
        .iter()
        .map(|(idx, item)| PickerLine {
            label: format!(" {} ", item.label),
            style: if *idx == selected {
                Style::default().fg(c.accent)
            } else {
                Style::default().fg(c.text)
            },
            selected: *idx == selected,
        })
        .collect();
    draw_searchable_popup(
        frame,
        area,
        " Actions ",
        filter,
        &lines,
        "Filter actions — Enter run · Esc cancel",
        if lines.is_empty() {
            "No matching actions"
        } else {
            ""
        },
    );
}

fn draw_favorites_picker(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    state: &crate::app::ListPickerState,
) {
    let c = colors(app);
    let indices = app.picker_indices();
    let list_height = centered_rect(70, 70, area).height.saturating_sub(6) as usize;
    let lines = build_picker_lines_from_strings(
        &indices,
        state.selected,
        list_height,
        |idx| {
            let f = &app.favorites[idx];
            format!("{}/{}/{}", f.kind, f.namespace, f.name)
        },
        |name| (name.to_string(), Style::default().fg(c.text)),
    );
    draw_searchable_popup(
        frame,
        area,
        " Favorites ",
        &state.search,
        &lines,
        "Jump to favorite resource",
        if indices.is_empty() {
            "No matching favorites"
        } else {
            ""
        },
    );
}

fn draw_editor_picker(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    state: &crate::app::ListPickerState,
) {
    let c = colors(app);
    let indices = app.picker_indices();
    let list_height = centered_rect(70, 70, area).height.saturating_sub(6) as usize;
    let lines = build_picker_lines_from_strings(
        &indices,
        state.selected,
        list_height,
        |idx| {
            let cand = &app.editor_candidates[idx];
            let status = if crate::app::editor_installed(cand.binary) {
                "installed"
            } else {
                "not installed"
            };
            format!("{} — {}", cand.label, status)
        },
        |name| {
            let installed = !name.contains("not installed");
            let style = if installed {
                Style::default().fg(c.text)
            } else {
                Style::default().fg(c.muted).add_modifier(Modifier::DIM)
            };
            (name.to_string(), style)
        },
    );
    draw_searchable_popup(
        frame,
        area,
        " Choose editor ",
        &state.search,
        &lines,
        "First run — pick an editor for apply/edit YAML. Installed ones shown brighter.",
        if indices.is_empty() {
            "No matching editors"
        } else {
            ""
        },
    );
}

fn draw_port_forward_list(frame: &mut Frame, area: Rect, app: &TuiApp, selected: usize) {
    let c = colors(app);
    let lines: Vec<Line> = if app.port_forward_entries.is_empty() {
        vec![Line::from(Span::styled(
            "No active port-forwards",
            Style::default().fg(c.muted),
        ))]
    } else {
        app.port_forward_entries
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let style = if idx == selected {
                    Style::default()
                        .fg(c.accent)
                        .add_modifier(Modifier::REVERSED)
                } else {
                    Style::default().fg(c.text)
                };
                Line::from(Span::styled(
                    format!(
                        " {}  127.0.0.1:{} → :{} ",
                        e.label, e.local_port, e.remote_port
                    ),
                    style,
                ))
            })
            .collect()
    };
    draw_simple_popup(
        frame,
        area,
        " Port-forwards ",
        &lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "Enter stop · Esc close",
        app,
    );
}

fn draw_settings(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    cursor: SettingsCursor,
    path_selected: usize,
) {
    let c = colors(app);
    let rows = [
        (
            SettingsCursor::NativePortForward,
            format!(
                "Native port-forward: {}",
                if app.use_native_port_forward {
                    "on"
                } else {
                    "off"
                }
            ),
        ),
        (
            SettingsCursor::ExternalLogs,
            format!(
                "Logs in external terminal: {}",
                if app.external_logs { "on" } else { "off" }
            ),
        ),
        (
            SettingsCursor::Theme,
            format!("Theme: {}", app.theme_mode.as_str()),
        ),
        (
            SettingsCursor::AddKubeconfigPath,
            "Add extra kubeconfig path…".into(),
        ),
        (
            SettingsCursor::ExtraKubeconfigList,
            if app.extra_kubeconfig_paths.is_empty() {
                "Extra kubeconfigs: (none)".into()
            } else {
                format!(
                    "Extra kubeconfigs (d delete): {}",
                    app.extra_kubeconfig_paths
                        .get(path_selected)
                        .cloned()
                        .unwrap_or_default()
                )
            },
        ),
        (
            SettingsCursor::Editor,
            format!(
                "Editor: {}",
                app.editor.as_deref().unwrap_or("$VISUAL/$EDITOR/vi")
            ),
        ),
    ];
    let body = rows
        .iter()
        .map(|(key, label)| {
            let mark = if *key == cursor { ">" } else { " " };
            format!("{mark} {label}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let _ = c;
    draw_simple_popup(
        frame,
        area,
        " Settings ",
        &body,
        "Enter toggle/add · Esc close",
        app,
    );
}

fn draw_help(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let body = "\
Navigation: h/l focus · j/k move · Tab kind · c context · n namespace
Resources: d detail · 1/2/3 Describe/Events/Metrics · drag anywhere to copy · L logs
Search: / table filter · detail focus+/ or Ctrl+f find · n/N next/prev match · y copy name
Logs: click focus · y/Ctrl+C copy · right-click/double-click copy line · f follow
Detail pan: Shift/Alt/Ctrl+wheel or wheel on ←→ bar · ←/→ or </> · title shows col
Ops: m actions · Ctrl+d delete · s scale · R restart · a apply · E edit
Shell/PF: e exec (this terminal) · p port-forward · P list PF
Favorites: f toggle · F jump · o overview
Clusters: [ ] cycle tabs · Ctrl+t add · Ctrl+w close
UI: t theme · , settings · ? help · q quit (Ctrl+C quits outside logs)";
    draw_simple_popup(frame, area, " Help ", body, "Esc/Enter close", app);
}

fn draw_context_picker(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    state: &crate::app::ListPickerState,
) {
    let active_context = if app.active_context.is_empty() {
        None
    } else {
        Some(app.active_context.as_str())
    };
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
    let active_namespace = if app.active_namespace.is_empty() {
        None
    } else {
        Some(app.active_namespace.as_str())
    };
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
    title: &str,
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
        title,
        &state.search,
        &lines,
        &format!("Pod: {pod_name}"),
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
    let namespace = if app.active_namespace.is_empty() {
        "?"
    } else {
        app.active_namespace.as_str()
    };
    let follow_label = if log.follow { "on" } else { "off" };
    let match_label = if log.search_query.is_empty() {
        String::new()
    } else if log.match_rows.is_empty() {
        " | no matches".to_string()
    } else {
        format!(" | match {}/{}", log.match_cursor + 1, log.match_rows.len())
    };

    let title = if log.pod_name.starts_with("svc/") {
        format!(
            " {} / {} | {} lines | follow: {follow_label}{match_label} ",
            namespace,
            log.pod_name,
            log.lines.len()
        )
    } else {
        format!(
            " {} / {} — container: {} | {} lines | follow: {follow_label}{match_label} ",
            namespace,
            log.pod_name,
            container,
            log.lines.len()
        )
    };

    let mut header_spans = vec![Span::styled(
        title,
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if let Some(err) = &log.error {
        header_spans.push(Span::raw(" | "));
        header_spans.push(Span::styled(err.clone(), Style::default().fg(ERROR)));
    }

    let panel_title = if log.pod_name.starts_with("svc/") {
        " Service pod logs "
    } else {
        " Pod logs "
    };
    let header = Paragraph::new(Line::from(header_spans))
        .block(Block::default().borders(Borders::ALL).title(panel_title));
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
        let max_scroll = wrapped.len().saturating_sub(visible_height);
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"));
        let mut scroll_state = ratatui::widgets::ScrollbarState::default()
            .content_length(max_scroll.saturating_add(1))
            .viewport_content_length(1)
            .position(scroll.min(max_scroll));
        frame.render_stateful_widget(scrollbar, log_chunks[1], &mut scroll_state);
    }

    let footer_text = if log.search_mode {
        "type search | Enter find | Esc clear/close search"
    } else {
        "Esc/q close | / search | n/N match | ↑/↓ scroll | f follow | click focus | y/Ctrl+C/right-click/double-click copy line"
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

/// Kinds in the exact order they render in the sidebar (categories top→bottom, then
/// Helm appended last). Navigation (`move_sidebar`/`next_kind`/`prev_kind`/
/// `kind_sidebar_index`) MUST use this — not `ResourceKind::ALL` — so arrow keys
/// match what's on screen. `ALL` differs (Helm before Crd) and causes the
/// Nodes↓→Helm(jumps to bottom)→↓→Custom(jumps up) bug.
pub fn sidebar_kinds() -> &'static [ResourceKind] {
    static SIDEBAR: &[ResourceKind] = &[
        ResourceKind::Pod,
        ResourceKind::Deployment,
        ResourceKind::StatefulSet,
        ResourceKind::Job,
        ResourceKind::CronJob,
        ResourceKind::Service,
        ResourceKind::Ingress,
        ResourceKind::NetworkPolicy,
        ResourceKind::PersistentVolumeClaim,
        ResourceKind::StorageClass,
        ResourceKind::Role,
        ResourceKind::RoleBinding,
        ResourceKind::ClusterRole,
        ResourceKind::ClusterRoleBinding,
        ResourceKind::ConfigMap,
        ResourceKind::Secret,
        ResourceKind::Namespace,
        ResourceKind::Node,
        ResourceKind::Crd,
        ResourceKind::HelmRelease,
    ];
    SIDEBAR
}

pub fn kind_sidebar_index(kind: ResourceKind) -> usize {
    sidebar_kinds().iter().position(|k| *k == kind).unwrap_or(0)
}

fn table_header_line(kind: ResourceKind, name_width: usize) -> String {
    match kind {
        ResourceKind::Pod => format!(
            "{} {} {} {} {}",
            fit("Name", name_width),
            fit("Ready", 7),
            fit("Status", 12),
            fit("Restarts", 8),
            fit("Age", 6),
        ),
        ResourceKind::Deployment => format!(
            "{} {} {} {} {}",
            fit("Name", name_width),
            fit("Ready", 8),
            fit("Up-to-date", 10),
            fit("Available", 9),
            fit("Age", 6),
        ),
        ResourceKind::CronJob => format!(
            "{} {} {} {} {}",
            fit("Name", name_width),
            fit("Schedule", 14),
            fit("Suspended", 9),
            fit("Active", 6),
            fit("Age", 6),
        ),
        _ => format!(
            "{} {} {} {}",
            fit("Name", name_width),
            fit("Ready", 8),
            fit("Status", 14),
            fit("Age", 6),
        ),
    }
}

fn format_row_line(kind: ResourceKind, row: &rl_core::ResourceRow, name_width: usize) -> String {
    match kind {
        ResourceKind::Pod => format!(
            "{} {} {} {} {}",
            fit(&row.name, name_width),
            fit(&row.ready, 7),
            fit(&row.status, 12),
            fit(&row.restarts, 8),
            fit(&row.age, 6),
        ),
        ResourceKind::Deployment => format!(
            "{} {} {} {} {}",
            fit(&row.name, name_width),
            fit(&row.ready, 8),
            fit(&row.up_to_date, 10),
            fit(&row.active, 9),
            fit(&row.age, 6),
        ),
        ResourceKind::CronJob => format!(
            "{} {} {} {} {}",
            fit(&row.name, name_width),
            fit(&row.schedule, 14),
            fit(&row.resumed, 9),
            fit(&row.active, 6),
            fit(&row.age, 6),
        ),
        _ => format!(
            "{} {} {} {}",
            fit(&row.name, name_width),
            fit(&row.ready, 8),
            fit(&row.status, 14),
            fit(&row.age, 6),
        ),
    }
}

/// Remaining width for the Name column after fixed columns + gaps.
fn name_column_width(kind: ResourceKind, total: usize) -> usize {
    let fixed = match kind {
        ResourceKind::Pod => 7 + 12 + 8 + 6 + 4, // gaps
        ResourceKind::Deployment => 8 + 10 + 9 + 6 + 4,
        ResourceKind::CronJob => 14 + 9 + 6 + 6 + 4,
        _ => 8 + 14 + 6 + 3,
    };
    total.saturating_sub(fixed).max(12)
}

fn fit(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        let mut out: String = chars.into_iter().collect();
        while out.chars().count() < width {
            out.push(' ');
        }
        out
    } else if width == 1 {
        "…".to_string()
    } else {
        let mut out: String = chars.into_iter().take(width - 1).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Navigation order must match render order, or arrow keys jump on screen
    /// (the Nodes↓→Helm(bottom)→↓→Custom(up) bug).
    #[test]
    fn sidebar_order_matches_render_order() {
        let mut rendered: Vec<ResourceKind> = Vec::new();
        for category in [
            ResourceCategory::Workloads,
            ResourceCategory::Network,
            ResourceCategory::Storage,
            ResourceCategory::Access,
            ResourceCategory::Config,
            ResourceCategory::Cluster,
            ResourceCategory::Custom,
        ] {
            rendered.extend(kinds_in_category(category));
        }
        rendered.push(ResourceKind::HelmRelease);
        let nav = sidebar_kinds();
        assert_eq!(nav.len(), rendered.len());
        for (i, k) in nav.iter().enumerate() {
            assert_eq!(*k, rendered[i], "mismatch at index {i}");
        }
    }

    #[test]
    fn kind_sidebar_indices_are_contiguous() {
        let kinds = sidebar_kinds();
        let indices: Vec<usize> = kinds.iter().map(|k| kind_sidebar_index(*k)).collect();
        let expected: Vec<usize> = (0..kinds.len()).collect();
        assert_eq!(indices, expected);
    }

    /// In `ALL`, HelmRelease(18) precedes Crd(19). In the sidebar, Crd renders above
    /// Helm, so navigation must use `sidebar_kinds` where Crd precedes HelmRelease.
    #[test]
    fn crd_precedes_helm_in_sidebar() {
        let kinds = sidebar_kinds();
        let crd = kinds.iter().position(|k| *k == ResourceKind::Crd).unwrap();
        let helm = kinds
            .iter()
            .position(|k| *k == ResourceKind::HelmRelease)
            .unwrap();
        assert!(crd < helm, "Crd must render above HelmRelease");
        assert_eq!(helm, kinds.len() - 1, "HelmRelease must be last");
    }
}
