//! Layout: top tab strip, full-width body (list or detail), footer.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table, TableState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::AppState;
use crate::docker::types::{
    ContainerRow, ImageRow, NetworkAttachedContainerRow, NetworkRow, VolumeContainerMountRow,
    VolumeRow,
};
use crate::events::{AppScreen, ContainerDetailTab, MainView};

fn footer_lines(state: &AppState) -> Vec<Line<'static>> {
    if let Some(name) = &state.volume_remove_pending {
        let t = truncate_chars(name, 48);
        return vec![Line::from(vec![Span::styled(
            format!(" Removing volume {t}… · please wait"),
            Style::default().fg(Color::DarkGray),
        )])];
    }
    if let Some(id) = &state.image_remove_pending {
        let t = truncate_chars(id, 48);
        return vec![Line::from(vec![Span::styled(
            format!(" Removing image {t}… · please wait"),
            Style::default().fg(Color::DarkGray),
        )])];
    }
    if let Some(id) = &state.container_action_pending {
        let t = truncate_chars(id, 12);
        return vec![Line::from(vec![Span::styled(
            format!(" Container action ({t})… · please wait"),
            Style::default().fg(Color::DarkGray),
        )])];
    }
    if state.image_delete_confirm {
        return vec![footer_single_line("Enter/y: delete · Esc/n/q: cancel")];
    }
    if state.screen == AppScreen::Detail && state.view == MainView::Volumes {
        return vec![volume_detail_footer_line(state)];
    }
    if state.screen == AppScreen::Detail && state.view == MainView::Networks {
        return vec![footer_single_line(
            "Esc/q: back · r: refresh · ↑↓ j/k PgUp/Dn: scroll containers",
        )];
    }
    if state.screen == AppScreen::Detail && state.view == MainView::Images {
        return vec![image_detail_footer_line(state)];
    }
    if state.screen == AppScreen::Detail && state.view == MainView::Containers {
        return vec![footer_single_line(
            "Esc/q: back · r: refresh · 1-4: section · ↑↓ j/k · PgUp/Dn · Home/End: scroll (Details, Logs, Inspect, Stats)",
        )];
    }
    if state.screen == AppScreen::Detail {
        return vec![footer_single_line("Esc/q: back to list")];
    }
    if state.screen == AppScreen::List && state.view == MainView::Containers {
        return vec![
            footer_single_line(&state.status_line),
            footer_single_line(
                "s: start · t: stop · p: pause · u: unpause · k: kill · d: remove",
            ),
        ];
    }
    vec![footer_single_line(&state.status_line)]
}

pub fn draw(frame: &mut Frame<'_>, state: &mut AppState) {
    let footer_lines_buf = footer_lines(state);
    let footer_h = footer_lines_buf.len().clamp(1, 2) as u16;

    let layout = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(footer_h),
    ]);

    let area = frame.area();

    let [tabs_area, body_area, footer_area] = area.layout(&layout);

    render_tab_strip(frame, tabs_area, state.view, state.screen);

    let table_rect = body_area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });

    match state.screen {
        AppScreen::Detail => {
            if state.view == MainView::Volumes {
                let content_lines = volume_detail_content_line_count(state) as u16;
                let desired = content_lines.saturating_add(2); // top + bottom border
                let detail_h = desired.min(table_rect.height.saturating_sub(5)).max(7);
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(detail_h), Constraint::Min(5)])
                    .split(table_rect);
                render_volume_detail(frame, chunks[0], state);
                render_volume_mount_users_table(frame, chunks[1], state);
            } else if state.view == MainView::Networks {
                if network_detail_show_containers_panel(state) {
                    let content_lines = network_detail_content_line_count() as u16;
                    let desired = content_lines.saturating_add(2); // top + bottom border
                    let detail_h = desired.min(table_rect.height.saturating_sub(5)).max(7);
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(detail_h), Constraint::Min(5)])
                        .split(table_rect);
                    render_network_detail(frame, chunks[0], state);
                    render_network_containers_in_network(frame, chunks[1], state);
                } else {
                    render_network_detail(frame, table_rect, state);
                }
            } else if state.view == MainView::Images {
                render_image_detail(frame, table_rect, state);
            } else if state.view == MainView::Containers {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(1)])
                    .split(table_rect);
                let subtab_area = chunks[0];
                let detail_area = chunks[1];
                render_container_detail_tab_strip(frame, subtab_area, state.container_detail_tab);
                match state.container_detail_tab {
                    ContainerDetailTab::Details => {
                        render_container_detail_details_tab(frame, detail_area, state);
                    }
                    ContainerDetailTab::Logs => {
                        render_container_logs_tab(frame, detail_area, state);
                    }
                    ContainerDetailTab::Inspect => {
                        render_container_inspect_tab(frame, detail_area, state);
                    }
                    ContainerDetailTab::Stats => {
                        render_container_stats_tab(frame, detail_area, state);
                    }
                }
            } else {
                render_detail_placeholder(frame, table_rect, state);
            }
        }
        AppScreen::List => match state.view {
            MainView::Dashboard => render_dashboard(frame, table_rect, state),
            MainView::Containers => render_containers_table(frame, table_rect, state),
            MainView::Images => render_images_table(frame, table_rect, state),
            MainView::Networks => render_networks_table(frame, table_rect, state),
            MainView::Volumes => render_volumes_table(frame, table_rect, state),
        },
    }

    render_footer_lines(frame, footer_area, footer_lines_buf);

    if let Some(ref msg) = state.error_banner {
        render_error_popup(frame, body_area, msg);
    }
    if state.image_delete_confirm {
        render_image_delete_confirm_popup(frame, body_area, state);
    }
}

fn footer_single_line(text: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(" {text}"),
        Style::default().fg(Color::DarkGray),
    )])
}

fn volume_detail_footer_line(state: &AppState) -> Line<'static> {
    let delete_enabled = state.volume_delete_enabled();
    let d_style = if delete_enabled {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    };
    Line::from(vec![
        Span::styled(" Esc/q: back · ", Style::default().fg(Color::DarkGray)),
        Span::styled("d: remove volume", d_style),
        Span::styled(" · r: refresh", Style::default().fg(Color::DarkGray)),
    ])
}

fn image_detail_footer_line(state: &AppState) -> Line<'static> {
    let delete_enabled = state.image_delete_enabled();
    let d_style = if delete_enabled {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    };
    Line::from(vec![
        Span::styled(" Esc/q: back · ", Style::default().fg(Color::DarkGray)),
        Span::styled("d: remove image", d_style),
        Span::styled(
            " · r: refresh · ↑↓ j/k · PgUp/Dn · Home/End: scroll",
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

fn render_tab_strip(frame: &mut Frame<'_>, area: Rect, view: MainView, screen: AppScreen) {
    let tabs = [
        (MainView::Dashboard, "1", "Dashboard"),
        (MainView::Containers, "2", "Containers"),
        (MainView::Images, "3", "Images"),
        (MainView::Networks, "4", "Networks"),
        (MainView::Volumes, "5", "Volumes"),
    ];

    // Contrast on dark tab bar; inactive must stay readable (no DIM on inactive — too faint).
    let inactive_fg = Color::Rgb(175, 182, 195);
    let active_fg = Color::Rgb(250, 252, 255);
    let active_bg = Color::Rgb(52, 88, 138);
    let sep_fg = Color::Rgb(65, 74, 88);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, (tab_view, num, name)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(sep_fg)));
        }
        let selected = *tab_view == view;
        let mut style = if selected {
            Style::default()
                .fg(active_fg)
                .bg(active_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(inactive_fg)
        };
        if screen == AppScreen::Detail && selected {
            style = style.add_modifier(Modifier::DIM);
        }
        let label = format!("{num} {name}");
        if selected {
            spans.push(Span::styled(format!(" {label} "), style));
        } else {
            spans.push(Span::styled(label, style));
        }
    }

    // Do not wrap tabs in a Block with borders here: tabs_row is only 1 cell tall, and a
    // bottom border would consume the only row and hide all tab text.
    let tabs_bar =
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(22, 28, 36)));
    frame.render_widget(tabs_bar, area);
}

/// Second-level tabs when viewing a container (keys `1`–`4` in detail).
fn render_container_detail_tab_strip(
    frame: &mut Frame<'_>,
    area: Rect,
    active: ContainerDetailTab,
) {
    let tabs = [
        (ContainerDetailTab::Details, "1", "Details"),
        (ContainerDetailTab::Logs, "2", "Logs"),
        (ContainerDetailTab::Inspect, "3", "Inspect"),
        (ContainerDetailTab::Stats, "4", "Stats"),
    ];

    let inactive_fg = Color::Rgb(175, 182, 195);
    let active_fg = Color::Rgb(250, 252, 255);
    let active_bg = Color::Rgb(52, 88, 138);
    let sep_fg = Color::Rgb(65, 74, 88);

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (i, (tab, num, name)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(sep_fg)));
        }
        let selected = *tab == active;
        let style = if selected {
            Style::default()
                .fg(active_fg)
                .bg(active_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(inactive_fg)
        };
        let label = format!("{num} {name}");
        if selected {
            spans.push(Span::styled(format!(" {label} "), style));
        } else {
            spans.push(Span::styled(label, style));
        }
    }

    let tabs_bar =
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(22, 28, 36)));
    frame.render_widget(tabs_bar, area);
}

fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return "…".into();
    }
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    format!("{}…", s.chars().take(take).collect::<String>())
}

/// Truncate to at most `max_width` **terminal columns** (Unicode display width), then `…`.
/// Use for boxed lines so content never spills past the right `│`.
fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 1 {
        return "…".into();
    }
    let ellipsis_w = "…".width().max(1);
    let budget = max_width.saturating_sub(ellipsis_w);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Parses `key=value` pairs from the volume list CSV (`"k1=v1, k2=v2"`).
fn parse_volume_label_pairs(s: &str) -> Option<Vec<(String, String)>> {
    if s.trim().is_empty() || s.trim() == "—" {
        return None;
    }
    let mut pairs = Vec::new();
    for part in s.split(", ") {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = part.split_once('=').map_or_else(
            || (part.to_string(), String::new()),
            |(k, v)| (k.to_string(), v.to_string()),
        );
        pairs.push((k, v));
    }
    if pairs.is_empty() { None } else { Some(pairs) }
}

fn pad_key_column(k: &str, width: usize) -> String {
    let n = k.chars().count();
    if n >= width {
        k.to_string()
    } else {
        format!("{}{}", k, " ".repeat(width - n))
    }
}

/// Inner text lines for the Volume Details paragraph (excludes block borders).
fn volume_detail_content_line_count(state: &AppState) -> usize {
    const BASE: usize = 4;
    let Some(i) = state.table_state.selected() else {
        return BASE + 1;
    };
    let Some(v) = state.volumes.get(i) else {
        return BASE + 1;
    };
    let label_lines = match parse_volume_label_pairs(&v.labels) {
        Some(pairs) if !pairs.is_empty() => pairs.len(),
        _ => 1,
    };
    BASE + label_lines
}

/// Inner text lines for the Network Details paragraph (excludes block borders).
fn network_detail_content_line_count() -> usize {
    10
}

/// The "Containers in network" panel is only for user-defined networks, not Docker system networks.
fn network_detail_show_containers_panel(state: &AppState) -> bool {
    let Some(i) = state.table_state.selected() else {
        return false;
    };
    state.networks.get(i).is_some_and(|n| !n.is_system)
}

fn render_volume_detail(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(i) = state.table_state.selected() else {
        return;
    };
    let Some(v) = state.volumes.get(i) else {
        return;
    };

    let content_cols = (area.width as usize).saturating_sub(4);
    let label_prefix = "Labels: ";
    let indent = label_prefix.chars().count();

    let mut lines: Vec<Line> = vec![
        dashboard_kv_line("ID", &v.name),
        dashboard_kv_line("Created", &v.created),
        dashboard_kv_line("Mount path", &v.mountpoint),
        dashboard_kv_line("Driver", &v.driver),
    ];

    if let Some(pairs) = parse_volume_label_pairs(&v.labels) {
        let key_width = pairs
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (i, (k, val)) in pairs.iter().enumerate() {
            let value_budget = content_cols.saturating_sub(indent + key_width + 1).max(1);
            let val_show = truncate_chars(val, value_budget);
            let key_col = pad_key_column(k, key_width);
            let row = format!("{key_col} {val_show}");
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(label_prefix.to_string(), Style::default().fg(Color::Yellow)),
                    Span::styled(row, Style::default().fg(Color::White)),
                ]));
            } else {
                let pad = " ".repeat(indent);
                lines.push(Line::from(vec![
                    Span::styled(pad, Style::default()),
                    Span::styled(row, Style::default().fg(Color::White)),
                ]));
            }
        }
    } else {
        lines.push(dashboard_kv_line("Labels", "—"));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Volume Details ");

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_volume_mount_users_table(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let header_cells = ["Container Name", "Mounted At", "Read-only"]
        .iter()
        .map(|h| Cell::from(*h).style(header_style));
    let header = Row::new(header_cells);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Containers using volume ");

    match &state.volume_mount_users {
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Loading…",
                    Style::default().fg(Color::DarkGray),
                )))
                .block(block),
                area,
            );
        }
        Some(rows) if rows.is_empty() => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No containers are using this volume.",
                    Style::default().fg(Color::DarkGray),
                )))
                .block(block),
                area,
            );
        }
        Some(rows) => {
            let row_count = rows.len();
            let data_rows: Vec<Row> = rows
                .iter()
                .map(|r: &VolumeContainerMountRow| {
                    Row::new(vec![
                        Cell::from(r.container_name.as_str()),
                        Cell::from(r.mounted_at.as_str()),
                        Cell::from(r.read_only.as_str()),
                    ])
                    .height(1)
                })
                .collect();
            let table = Table::new(
                data_rows,
                [
                    Constraint::Min(18),
                    Constraint::Min(22),
                    Constraint::Length(12),
                ],
            )
            .header(header)
            .block(block);

            render_table_with_scrollbar(
                frame,
                area,
                table,
                &mut state.volume_mount_users_table_state,
                row_count,
                &mut state.volume_mount_users_viewport_rows,
                true,
            );
        }
    }
}

fn render_network_detail(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(i) = state.table_state.selected() else {
        return;
    };
    let Some(n) = state.networks.get(i) else {
        return;
    };

    let max_w = (area.width as usize).saturating_sub(12).max(24);
    let t = |s: &str| truncate_chars(s, max_w);

    let lines: Vec<Line> = vec![
        dashboard_kv_line("Name", &t(&n.name)),
        dashboard_kv_line("ID", &t(&n.id)),
        dashboard_kv_line("Driver", &t(&n.driver)),
        dashboard_kv_line("Scope", &t(&n.scope)),
        dashboard_kv_line("Attachable", &n.attachable),
        dashboard_kv_line("Internal", &n.internal),
        dashboard_kv_line("IPV4 Subnet", &t(&n.ipv4_subnet)),
        dashboard_kv_line("IPV4 Gateway", &t(&n.ipv4_gateway)),
        dashboard_kv_line("IPV6 Subnet", &t(&n.ipv6_subnet)),
        dashboard_kv_line("IPV6 Gateway", &t(&n.ipv6_gateway)),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Network Details ");

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_network_containers_in_network(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let header_cells = [
        "Container Name",
        "IPv4 Address",
        "IPv6 Address",
        "MacAddress",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(header_style));
    let header = Row::new(header_cells);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Containers in network ");

    match &state.network_containers {
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Loading…",
                    Style::default().fg(Color::DarkGray),
                )))
                .block(block),
                area,
            );
        }
        Some(rows) if rows.is_empty() => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No containers are attached to this network.",
                    Style::default().fg(Color::DarkGray),
                )))
                .block(block),
                area,
            );
        }
        Some(rows) => {
            let row_count = rows.len();
            let data_rows: Vec<Row> = rows
                .iter()
                .map(|r: &NetworkAttachedContainerRow| {
                    Row::new(vec![
                        Cell::from(r.container_name.as_str()),
                        Cell::from(r.ipv4_address.as_str()),
                        Cell::from(r.ipv6_address.as_str()),
                        Cell::from(r.mac_address.as_str()),
                    ])
                    .height(1)
                })
                .collect();

            let table = Table::new(
                data_rows,
                [
                    Constraint::Min(18),
                    Constraint::Min(14),
                    Constraint::Min(18),
                    Constraint::Min(17),
                ],
            )
            .header(header)
            .block(block);

            render_table_with_scrollbar(
                frame,
                area,
                table,
                &mut state.network_containers_table_state,
                row_count,
                &mut state.network_containers_viewport_rows,
                true,
            );
        }
    }
}

/// Portainer-style image detail: fixed tags header, one scroll (Image + Dockerfile + Layers text table).
fn render_image_detail(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let h = area.height;
    // Header: minimum 2 lines; 1 line margin below when there is room for header + gap + ≥1 scroll line.
    let header_h = 2u16.min(h);
    let remaining_after_header = h.saturating_sub(header_h);
    let margin_h = if remaining_after_header >= 2 { 1 } else { 0 };
    let scroll_h = remaining_after_header.saturating_sub(margin_h);

    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: header_h,
    };
    let scroll_area = Rect {
        x: area.x,
        y: area.y + header_h + margin_h,
        width: area.width,
        height: scroll_h,
    };

    render_image_detail_header(frame, header_area, state);

    if scroll_h == 0 {
        return;
    }

    let full_width = (scroll_area.width as usize).saturating_sub(2).max(12);
    let lines = image_detail_panel_lines(state, full_width);
    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(scroll_area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.image_detail_scroll = state.image_detail_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.image_detail_scroll as u16, 0));

    frame.render_widget(paragraph, scroll_area);

    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.image_detail_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: scroll_area.x.saturating_add(1),
        y: scroll_area.y,
        width: scroll_area.width,
        height: scroll_area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

/// Logs tab: stdout/stderr text (tail-limited when fetched).
fn render_container_logs_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let lines: Vec<Line<'static>> = match &state.container_logs {
        None => vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(text) if text.is_empty() => vec![Line::from(Span::styled(
            "No log output.",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(text) => text
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect(),
    };
    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.container_logs_scroll = state.container_logs_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.container_logs_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.container_logs_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width,
        height: area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

/// Inspect tab: pretty-printed `docker inspect` JSON.
fn render_container_inspect_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let lines: Vec<Line<'static>> = match &state.container_inspect_json {
        None => vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(text) if text.is_empty() => vec![Line::from(Span::styled(
            "(empty)",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(text) => text
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect(),
    };
    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.container_inspect_json_scroll = state.container_inspect_json_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.container_inspect_json_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.container_inspect_json_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width,
        height: area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

/// Stats tab: one-shot CPU, memory, network, block I/O, PIDs (`docker stats` snapshot).
fn render_container_stats_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let lines: Vec<Line<'static>> = match &state.container_stats {
        None => vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(snapshot) if snapshot.lines.is_empty() => vec![Line::from(Span::styled(
            "No stats data.",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(snapshot) => snapshot
            .lines
            .iter()
            .map(|l| {
                if l.is_empty() {
                    Line::default()
                } else {
                    Line::from(Span::raw(l.clone()))
                }
            })
            .collect(),
    };
    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.container_stats_scroll = state.container_stats_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.container_stats_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.container_stats_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width,
        height: area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

/// Details tab: one scroll — status, details, volumes, networks (ASCII box panels).
fn render_container_detail_details_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let full_width = (area.width as usize).saturating_sub(2).max(12);
    let lines = container_detail_panel_lines(state, full_width);
    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.container_detail_scroll = state.container_detail_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.container_detail_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.container_detail_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width,
        height: area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

fn container_detail_panel_lines(state: &AppState, full_width: usize) -> Vec<Line<'static>> {
    let text_cols = full_width.saturating_sub(4).max(8);
    let Some(i) = state.table_state.selected() else {
        return bordered_panel_from_inner_lines(
            "Container details",
            vec![Line::from(Span::styled(
                "No container selected.",
                Style::default().fg(Color::DarkGray),
            ))],
            full_width,
        );
    };
    let Some(c) = state.containers.get(i) else {
        return bordered_panel_from_inner_lines(
            "Container details",
            vec![Line::from(Span::styled(
                "No container selected.",
                Style::default().fg(Color::DarkGray),
            ))],
            full_width,
        );
    };

    let mut lines = Vec::new();
    lines.extend(bordered_panel_from_inner_lines(
        "Container status",
        container_status_inner_lines(state, c, text_cols),
        full_width,
    ));
    lines.push(Line::default());
    lines.extend(bordered_panel_from_inner_lines(
        "Container details",
        container_detail_kv_inner_lines(state, c, text_cols),
        full_width,
    ));
    lines.push(Line::default());
    lines.extend(bordered_panel_from_inner_lines(
        "Volumes",
        container_volumes_inner_lines(state, text_cols),
        full_width,
    ));
    lines.push(Line::default());
    lines.extend(bordered_panel_from_inner_lines(
        "Connected networks",
        container_networks_inner_lines(state, text_cols),
        full_width,
    ));
    lines
}

/// Mounts as columnar text (same style as image layers — scrolls with the main view).
fn container_volumes_inner_lines(state: &AppState, text_cols: usize) -> Vec<Line<'static>> {
    let Some(ins) = &state.container_inspect else {
        return vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    if ins.mounts.is_empty() {
        return vec![Line::from(Span::styled(
            "No mounts.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    const T_W: usize = 8;
    const RW_W: usize = 4;
    let gap = 3usize;
    let rem = text_cols.saturating_sub(T_W + RW_W + gap);
    let s_w = (rem / 2).max(6);
    let d_w = rem.saturating_sub(s_w).max(6);

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let header_str = format!(
        "{:<tw$} {:<sw$} {:<dw$} {:<rw$}",
        "Type",
        "Source",
        "Destination",
        "RW",
        tw = T_W,
        sw = s_w,
        dw = d_w,
        rw = RW_W,
    );
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        truncate_to_display_width(&header_str, text_cols),
        header_style,
    ))];

    for r in &ins.mounts {
        let ty = truncate_to_display_width(&r.mount_type, T_W);
        let src = truncate_to_display_width(&r.source, s_w);
        let dst = truncate_to_display_width(&r.destination, d_w);
        let rw = truncate_to_display_width(&r.rw, RW_W);
        let row = format!(
            "{:<tw$} {:<sw$} {:<dw$} {:<rw$}",
            ty,
            src,
            dst,
            rw,
            tw = T_W,
            sw = s_w,
            dw = d_w,
            rw = RW_W,
        );
        lines.push(Line::from(Span::styled(
            truncate_to_display_width(&row, text_cols),
            Style::default().fg(Color::White),
        )));
    }

    lines
}

/// Network endpoints as columnar text (scrolls with the main view).
fn container_networks_inner_lines(state: &AppState, text_cols: usize) -> Vec<Line<'static>> {
    let Some(ins) = &state.container_inspect else {
        return vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    if ins.networks.is_empty() {
        return vec![Line::from(Span::styled(
            "No network attachments.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    const ID_W: usize = 12;
    const V4_W: usize = 14;
    let gap = 4usize;
    let rem = text_cols.saturating_sub(ID_W + V4_W + gap);
    let n_w = (rem * 28 / 100).max(10);
    let v6_w = (rem * 32 / 100).max(8);
    let mac_w = rem.saturating_sub(n_w + v6_w).max(8);

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let header_str = format!(
        "{:<nw$} {:<idw$} {:<v4$} {:<v6$} {:<mw$}",
        "Network",
        "Network ID",
        "IPv4",
        "IPv6",
        "MAC",
        nw = n_w,
        idw = ID_W,
        v4 = V4_W,
        v6 = v6_w,
        mw = mac_w,
    );
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        truncate_to_display_width(&header_str, text_cols),
        header_style,
    ))];

    for r in &ins.networks {
        let net = truncate_to_display_width(&r.network_name, n_w);
        let nid = truncate_to_display_width(&r.network_id, ID_W);
        let v4 = truncate_to_display_width(&r.ipv4, V4_W);
        let v6 = truncate_to_display_width(&r.ipv6, v6_w);
        let mac = truncate_to_display_width(&r.mac_address, mac_w);
        let row = format!(
            "{:<nw$} {:<idw$} {:<v4$} {:<v6$} {:<mw$}",
            net,
            nid,
            v4,
            v6,
            mac,
            nw = n_w,
            idw = ID_W,
            v4 = V4_W,
            v6 = v6_w,
            mw = mac_w,
        );
        lines.push(Line::from(Span::styled(
            truncate_to_display_width(&row, text_cols),
            Style::default().fg(Color::White),
        )));
    }

    lines
}

fn container_status_inner_lines(
    state: &AppState,
    c: &ContainerRow,
    text_cols: usize,
) -> Vec<Line<'static>> {
    let start_time = match &state.container_inspect {
        Some(s) => s.started_at.as_str(),
        None => {
            if state.container_inspect_id.as_deref() == Some(c.id.as_str()) {
                "Loading…"
            } else {
                "—"
            }
        }
    };
    vec![
        dashboard_kv_line_max("State", &c.state, text_cols),
        dashboard_kv_line_max("Status", &c.status, text_cols),
        dashboard_kv_line_max(
            "Running",
            if c.running { "Yes" } else { "No" },
            text_cols,
        ),
        dashboard_kv_line_max("Created", &c.created, text_cols),
        dashboard_kv_line_max("Start time", start_time, text_cols),
    ]
}

fn container_detail_kv_inner_lines(
    state: &AppState,
    c: &ContainerRow,
    text_cols: usize,
) -> Vec<Line<'static>> {
    let id_show = if c.full_id.is_empty() {
        "—"
    } else {
        c.full_id.as_str()
    };
    let published_ports = match &state.container_inspect {
        Some(s) => s.published_ports.as_str(),
        None => {
            if state.container_inspect_id.as_deref() == Some(c.id.as_str()) {
                "Loading…"
            } else {
                "—"
            }
        }
    };
    vec![
        dashboard_kv_line_max("ID", id_show, text_cols),
        dashboard_kv_line_max("Names", &c.names, text_cols),
        dashboard_kv_line_max("Image", &c.image, text_cols),
        dashboard_kv_line_max("Stack", &c.stack, text_cols),
        dashboard_kv_line_max("Published ports", published_ports, text_cols),
    ]
}

/// Fixed header: repo tags (full list after inspect, or selected row tag while loading).
fn render_image_detail_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let tag_bar_bg = Color::Rgb(22, 28, 36);
    let label_style = Style::default()
        .fg(Color::Rgb(175, 182, 195))
        .add_modifier(Modifier::BOLD);
    let value_style = Style::default().fg(Color::Rgb(250, 252, 255));

    let tags_text = image_detail_tags_display(state);
    let header = Paragraph::new(Line::from(vec![
        Span::styled("Tags: ", label_style),
        Span::styled(tags_text, value_style),
    ]))
    .wrap(Wrap { trim: true })
    .block(
        Block::default()
            .style(Style::default().bg(tag_bar_bg))
            .padding(Padding {
                left: 1,
                right: 1,
                top: 0,
                bottom: 0,
            }),
    );

    frame.render_widget(header, area);
}

fn image_detail_tags_display(state: &AppState) -> String {
    if let Some(d) = &state.image_detail {
        return d.tags.clone();
    }
    if let Some(i) = state.table_state.selected() {
        if let Some(im) = state.images.get(i) {
            return im.tag.clone();
        }
    }
    "—".into()
}

/// All image detail panels in one scroll: Image + Dockerfile + Layers (columnar text, no table scrollbar).
fn image_detail_panel_lines(state: &AppState, full_width: usize) -> Vec<Line<'static>> {
    let text_cols = full_width.saturating_sub(4).max(8);
    let mut lines = Vec::new();
    lines.extend(bordered_panel_from_inner_lines(
        "Image details",
        image_details_inner_lines(state, text_cols),
        full_width,
    ));
    lines.push(Line::default());
    lines.extend(bordered_panel_from_inner_lines(
        "Dockerfile details",
        dockerfile_details_inner_lines(state, text_cols),
        full_width,
    ));
    lines.push(Line::default());
    lines.extend(bordered_panel_from_inner_lines(
        "Image layers",
        image_layers_inner_lines(state, text_cols),
        full_width,
    ));
    lines
}

/// Order / Size / Layer as fixed-width text rows (no ratatui Table — scrolls with the page).
fn image_layers_inner_lines(state: &AppState, text_cols: usize) -> Vec<Line<'static>> {
    let Some(d) = &state.image_detail else {
        return vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    if d.layers.is_empty() {
        return vec![Line::from(Span::styled(
            "No layer history.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    const ORDER_W: usize = 6;
    const SIZE_W: usize = 12;
    let layer_w = text_cols.saturating_sub(ORDER_W + 1 + SIZE_W + 1).max(4);

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let header_str = format!("{:<6} {:<12} {}", "Order", "Size", "Layer");
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        truncate_to_display_width(&header_str, text_cols),
        header_style,
    ))];

    for r in &d.layers {
        let size_show = truncate_to_display_width(&r.size, SIZE_W);
        let layer_show = truncate_to_display_width(&r.layer, layer_w);
        let row = format!("{:<6} {:12} {}", r.order, size_show, layer_show);
        lines.push(Line::from(Span::styled(
            truncate_to_display_width(&row, text_cols),
            Style::default().fg(Color::White),
        )));
    }

    lines
}

/// Dockerfile-style lines from image `Config` (CMD, ENTRYPOINT, ENV, …).
fn dockerfile_details_inner_lines(state: &AppState, text_cols: usize) -> Vec<Line<'static>> {
    let Some(d) = &state.image_detail else {
        return vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let mut lines: Vec<Line<'static>> = vec![
        dashboard_kv_line_max("CMD", &d.cmd, text_cols),
        dashboard_kv_line_max("ENTRYPOINT", &d.entrypoint, text_cols),
    ];
    push_env_lines_like_labels(&mut lines, &d.env, text_cols);
    lines.extend([
        dashboard_kv_line_max("WORKDIR", &d.working_dir, text_cols),
        dashboard_kv_line_max("USER", &d.user, text_cols),
        dashboard_kv_line_max("EXPOSE", &d.expose, text_cols),
        dashboard_kv_line_max("VOLUME", &d.volume, text_cols),
        dashboard_kv_line_max("SHELL", &d.shell, text_cols),
        dashboard_kv_line_max("ONBUILD", &d.on_build, text_cols),
    ]);
    lines
}

fn push_env_lines_like_labels(lines: &mut Vec<Line<'static>>, env: &[String], text_cols: usize) {
    let label_prefix = "ENV: ";
    let indent = label_prefix.chars().count();
    if env.is_empty() {
        lines.push(dashboard_kv_line_max("ENV", "—", text_cols));
        return;
    }
    for (i, e) in env.iter().enumerate() {
        let budget = text_cols.saturating_sub(indent).max(1);
        let row = truncate_chars(e, budget);
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(label_prefix.to_string(), Style::default().fg(Color::Yellow)),
                Span::styled(row, Style::default().fg(Color::White)),
            ]));
        } else {
            let pad = " ".repeat(indent);
            lines.push(Line::from(vec![
                Span::styled(pad, Style::default()),
                Span::styled(row, Style::default().fg(Color::White)),
            ]));
        }
    }
}

/// Key/value lines for the Image details panel (matches Volume Details label layout).
fn image_details_inner_lines(state: &AppState, text_cols: usize) -> Vec<Line<'static>> {
    let Some(d) = &state.image_detail else {
        return vec![Line::from(Span::styled(
            "Loading…",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let mut lines: Vec<Line<'static>> = vec![
        dashboard_kv_line_max("ID", &d.id, text_cols),
        dashboard_kv_line_max("Size", &d.size, text_cols),
        dashboard_kv_line_max("Created", &d.created, text_cols),
        dashboard_kv_line_max("Build", &d.build, text_cols),
    ];

    let label_prefix = "Labels: ";
    let indent = label_prefix.chars().count();
    if let Some(pairs) = parse_volume_label_pairs(&d.labels) {
        let key_width = pairs
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (i, (k, val)) in pairs.iter().enumerate() {
            let value_budget = text_cols.saturating_sub(indent + key_width + 1).max(1);
            let val_show = truncate_chars(val, value_budget);
            let key_col = pad_key_column(k, key_width);
            let row = format!("{key_col} {val_show}");
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled(label_prefix.to_string(), Style::default().fg(Color::Yellow)),
                    Span::styled(row, Style::default().fg(Color::White)),
                ]));
            } else {
                let pad = " ".repeat(indent);
                lines.push(Line::from(vec![
                    Span::styled(pad, Style::default()),
                    Span::styled(row, Style::default().fg(Color::White)),
                ]));
            }
        }
    } else {
        lines.push(dashboard_kv_line_max("Labels", "—", text_cols));
    }

    lines
}

fn dashboard_kv_line_max(label: &str, value: &str, max_width: usize) -> Line<'static> {
    let prefix = format!("{label}: ");
    let vw = max_width.saturating_sub(prefix.chars().count()).max(1);
    let v = truncate_chars(value, vw);
    Line::from(vec![
        Span::styled(prefix, Style::default().fg(Color::Yellow)),
        Span::styled(v, Style::default().fg(Color::White)),
    ])
}

fn bordered_panel_from_inner_lines(
    title: &str,
    inner: Vec<Line<'static>>,
    full_width: usize,
) -> Vec<Line<'static>> {
    let box_fg = Color::Rgb(120, 140, 170);
    let inner_w = full_width.saturating_sub(2);
    let content_w = inner_w.saturating_sub(2);

    let title_part = format!(" {title} ");
    let fill_count = inner_w.saturating_sub(title_part.chars().count());
    let top = format!("┌{}{}┐", title_part, "─".repeat(fill_count.max(0)));

    let mut out = vec![Line::from(Span::styled(top, Style::default().fg(box_fg)))];

    for mut line in inner {
        if line.width() > content_w {
            let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let style = line.spans.first().map(|s| s.style).unwrap_or_default();
            line = Line::from(vec![Span::styled(
                truncate_to_display_width(&plain, content_w),
                style,
            )]);
        }
        let w = line.width();
        let pad = content_w.saturating_sub(w);
        let mut spans = vec![Span::styled("│ ", box_fg)];
        spans.extend(line);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        spans.push(Span::styled(" │", box_fg));
        out.push(Line::from(spans));
    }

    let bottom = format!("└{}┘", "─".repeat(inner_w));
    out.push(Line::from(Span::styled(
        bottom,
        Style::default().fg(box_fg),
    )));
    out
}

fn render_detail_placeholder(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let (title, intro) = match state.view {
        MainView::Dashboard => ("Details", "Nothing to show."),
        MainView::Containers => (
            "Container",
            "Inspect and lifecycle actions will appear here.",
        ),
        MainView::Images => ("Image", "Image metadata and actions will appear here."),
        MainView::Networks => ("Network", "Network details will appear here."),
        MainView::Volumes => ("Volume", "Volume details will appear here."),
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Selection: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                state.detail_selection_label(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::default(),
        Line::from(Span::styled(intro, Style::default().fg(Color::DarkGray))),
    ];

    if state.view == MainView::Containers {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Planned: s start · t stop · k kill · r restart · p pause · u resume · d remove (confirm)",
            Style::default().fg(Color::Rgb(90, 100, 120)),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .border_style(Style::default().fg(Color::Rgb(80, 100, 130)));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn dashboard_kv_line(label: &str, value: &str) -> Line<'static> {
    let label_s = format!("{label}: ");
    let value_s = value.to_string();
    Line::from(vec![
        Span::styled(label_s, Style::default().fg(Color::Yellow)),
        Span::styled(value_s, Style::default().fg(Color::White)),
    ])
}

fn section_title(title: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        title.to_string(),
        Style::default()
            .fg(Color::Rgb(200, 220, 240))
            .add_modifier(Modifier::BOLD),
    )])
}

fn render_dashboard(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let Some(d) = &state.dashboard else {
        let p = Paragraph::new("Loading dashboard…")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default());
        frame.render_widget(p, area);
        return;
    };

    let summary_lines: Vec<Line> = vec![
        dashboard_kv_line("Environment", &d.environment),
        dashboard_kv_line("Endpoint", &d.endpoint_url),
        dashboard_kv_line("Containers", &d.containers_total),
        dashboard_kv_line("Volumes", &d.volumes_total),
        dashboard_kv_line("Networks", &d.networks_total),
        dashboard_kv_line("Stacks", &d.stacks),
        dashboard_kv_line("Images", &d.images_total),
    ];
    let host_lines: Vec<Line> = vec![
        dashboard_kv_line("Hostname", &d.hostname),
        dashboard_kv_line("OS", &d.operating_system),
        dashboard_kv_line("Kernel", &d.kernel_version),
        dashboard_kv_line("CPUs", &d.cpu_total),
        dashboard_kv_line("Memory", &d.memory_total),
    ];
    let engine_lines: Vec<Line> = vec![
        dashboard_kv_line("Version", &d.engine_version),
        dashboard_kv_line("Root directory", &d.docker_root),
        dashboard_kv_line("Storage driver", &d.storage_driver),
        dashboard_kv_line("Logging driver", &d.logging_driver),
        dashboard_kv_line("Volume plugins", &d.volume_plugins),
        dashboard_kv_line("Network plugins", &d.network_plugins),
    ];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(section_title("Summary"));
    lines.extend(summary_lines);
    lines.push(Line::default());
    lines.push(section_title("Host"));
    lines.extend(host_lines);
    lines.push(Line::default());
    lines.push(section_title("Engine"));
    lines.extend(engine_lines);

    let total_lines = lines.len().max(1);

    let block = Block::default();
    let inner = block.inner(area);
    let inner_h = inner.height as usize;

    let max_scroll = total_lines.saturating_sub(inner_h);
    state.dashboard_scroll = state.dashboard_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((state.dashboard_scroll as u16, 0));

    frame.render_widget(paragraph, area);

    // Match `render_table_with_scrollbar`: `content_length` is scroll-position count
    // (`max_scroll + 1`), not total lines — otherwise the thumb length is wrong.
    let content_len = if max_scroll == 0 { 1 } else { max_scroll + 1 };
    let position = state.dashboard_scroll.min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(inner_h.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width,
        height: area.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

/// Modal error overlay ([`Clear`] + bordered block), same pattern as the
/// [Ratatui popup example](https://ratatui.rs/examples/apps/popup/).
fn render_error_popup(frame: &mut Frame<'_>, area: Rect, msg: &str) {
    let vertical = Layout::vertical([
        Constraint::Percentage(18),
        Constraint::Length(16),
        Constraint::Percentage(18),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage(6),
        Constraint::Percentage(88),
        Constraint::Percentage(6),
    ])
    .split(vertical[1]);
    let popup_area = horizontal[1];

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(Span::styled(" Error ", Style::default().fg(Color::Red)));

    let text = format!(
        "{msg}\n\nEnter or Space: dismiss · On the main list, q or Esc exits the app"
    );
    let p = Paragraph::new(text).wrap(Wrap { trim: true }).block(block);

    frame.render_widget(p, popup_area);
}

fn render_image_delete_confirm_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let vertical = Layout::vertical([
        Constraint::Percentage(18),
        Constraint::Length(16),
        Constraint::Percentage(18),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage(6),
        Constraint::Percentage(88),
        Constraint::Percentage(6),
    ])
    .split(vertical[1]);
    let popup_area = horizontal[1];

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " Delete image? ",
            Style::default().fg(Color::Yellow),
        ));

    let id_line = state.image_detail_ref.as_deref().unwrap_or("—");
    let tags = state
        .image_detail
        .as_ref()
        .map(|d| d.tags.as_str())
        .unwrap_or("—");
    let text = format!(
        "This cannot be undone.\n\nID: {id_line}\nTags: {tags}\n\nEnter or y: delete · Esc, n, or q: cancel"
    );
    let p = Paragraph::new(text).wrap(Wrap { trim: true }).block(block);

    frame.render_widget(p, popup_area);
}

fn render_containers_table(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let row_count = state.containers.len();
    let header_cells = ["NAME", "IMAGE", "STACK", "STATE", "STATUS"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header = Row::new(header_cells);

    let rows: Vec<Row> = state
        .containers
        .iter()
        .map(|c: &ContainerRow| {
            let row = Row::new(vec![
                Cell::from(c.names.as_str()),
                Cell::from(c.image.as_str()),
                Cell::from(c.stack.as_str()),
                Cell::from(c.state.as_str()),
                Cell::from(c.status.as_str()),
            ]);

            if c.running {
                row
            } else {
                row.style(Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Min(18),
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(32),
        ],
    )
    .header(header)
    .block(Block::default().padding(Padding {
        top: 0,
        bottom: 0,
        left: 1,
        right: 1,
    }))
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(45, 55, 72))
            .add_modifier(Modifier::BOLD),
    );

    render_table_with_scrollbar(
        frame,
        area,
        table,
        &mut state.table_state,
        row_count,
        &mut state.table_viewport_rows,
        false,
    );
}

fn render_images_table(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let row_count = state.images.len();
    let header_cells = ["IMAGE ID", "TAG", "SIZE", "AGE"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells);

    let rows: Vec<Row> = state
        .images
        .iter()
        .map(|im: &ImageRow| {
            Row::new(vec![
                Cell::from(im.id.as_str()),
                Cell::from(im.tag.as_str()),
                Cell::from(im.size.as_str()),
                Cell::from(im.created.as_str()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Min(22),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(22),
        ],
    )
    .header(header)
    .block(Block::default().padding(Padding {
        top: 0,
        bottom: 0,
        left: 1,
        right: 1,
    }))
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(45, 55, 72))
            .add_modifier(Modifier::BOLD),
    );

    render_table_with_scrollbar(
        frame,
        area,
        table,
        &mut state.table_state,
        row_count,
        &mut state.table_viewport_rows,
        false,
    );
}

fn render_networks_table(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let row_count = state.networks.len();
    let header_cells = [
        "NAME",
        "DRIVER",
        "STACK",
        "IPV4 Subnet",
        "IPV4 Gateway",
        "IPV6 Subnet",
        "IPV6 Gateway",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells);

    let rows: Vec<Row> = state
        .networks
        .iter()
        .map(|n: &NetworkRow| {
            let name_cell = if n.is_system {
                Cell::from(Span::styled(
                    n.name.as_str(),
                    Style::default().fg(Color::Blue),
                ))
            } else {
                Cell::from(n.name.as_str())
            };
            Row::new(vec![
                name_cell,
                Cell::from(n.driver.as_str()),
                Cell::from(n.stack.as_str()),
                Cell::from(n.ipv4_subnet.as_str()),
                Cell::from(n.ipv4_gateway.as_str()),
                Cell::from(n.ipv6_subnet.as_str()),
                Cell::from(n.ipv6_gateway.as_str()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Min(12),
            Constraint::Min(14),
            Constraint::Min(12),
            Constraint::Min(14),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .block(Block::default().padding(Padding {
        top: 0,
        bottom: 0,
        left: 1,
        right: 1,
    }))
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(45, 55, 72))
            .add_modifier(Modifier::BOLD),
    );

    render_table_with_scrollbar(
        frame,
        area,
        table,
        &mut state.table_state,
        row_count,
        &mut state.table_viewport_rows,
        false,
    );
}

fn render_volumes_table(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let header_cells = ["NAME", "DRIVER", "MOUNT_POINT", "STACK", "CREATED"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header = Row::new(header_cells);

    let rows: Vec<Row> = state
        .volumes
        .iter()
        .map(|v: &VolumeRow| {
            let row = Row::new(vec![
                Cell::from(v.name.as_str()),
                Cell::from(v.driver.as_str()),
                Cell::from(v.mountpoint.as_str()),
                Cell::from(v.stack.as_str()),
                Cell::from(v.created.as_str()),
            ]);

            // Unused: Docker RefCount == 0 (`VolumeRow::unused`); dim, not hide.
            if v.unused {
                row.style(Style::default().add_modifier(Modifier::DIM))
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Min(18),
            Constraint::Min(12),
            Constraint::Length(22),
        ],
    )
    .header(header)
    .block(Block::default().padding(Padding {
        top: 0,
        bottom: 0,
        left: 1,
        right: 1,
    }))
    .row_highlight_style(
        Style::default()
            .bg(Color::Rgb(45, 55, 72))
            .add_modifier(Modifier::BOLD),
    );

    let row_count = state.volumes.len();

    render_table_with_scrollbar(
        frame,
        area,
        table,
        &mut state.table_state,
        row_count,
        &mut state.table_viewport_rows,
        false,
    );
}

/// Renders the table in `area` and paints a vertical scrollbar on the block’s right border
/// (last column), using [`TableState::offset`] after the table updates it.
fn render_table_with_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    table: Table<'_>,
    table_state: &mut TableState,
    row_count: usize,
    viewport_rows_out: &mut usize,
    bordered: bool,
) {
    frame.render_stateful_widget(table, area, table_state);

    if row_count == 0 {
        return;
    }

    let viewport_lines = usize::from(area.height.saturating_sub(1));
    *viewport_rows_out = viewport_lines.max(1);

    // ScrollbarState: `content_length` is scroll-position count (`max_offset + 1`), same as
    // [`render_dashboard`]. `TableState::offset` is the first visible data row index.
    let max_offset = row_count.saturating_sub(viewport_lines);
    let content_len = if max_offset == 0 { 1 } else { max_offset + 1 };
    let position = table_state.offset().min(content_len.saturating_sub(1));

    let mut scrollbar_state = ScrollbarState::new(content_len)
        .position(position)
        .viewport_content_length(viewport_lines.max(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);

    let scrollbar_area = if bordered {
        Rect {
            x: area.x,
            y: area.y.saturating_add(2),
            width: area.width,
            height: area.height.saturating_sub(3),
        }
    } else {
        Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        }
    };

    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

fn render_footer_lines(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    let p = Paragraph::new(lines).style(Style::default().bg(Color::Rgb(22, 22, 28)));
    frame.render_widget(p, area);
}
