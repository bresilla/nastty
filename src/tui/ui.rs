//! Rendering for the main view. Pure functions over `App` state, so they
//! can be exercised with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Clear, Gauge, Paragraph, Row, Sparkline, Table, TableState, Tabs};
use serde_json::Value;

use super::app::{App, Confirm, Form, Modal, TABS, UsersSelection};
use super::theme;

const TAB_ICONS: [&str; 11] = ["⌂", "⛁", "▤", "▦", "◷", "⇄", "🗀", "☰", "◉", "◍", "⚙"];

pub(super) fn render_app(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(f.area());

    render_tabs(f, chunks[0], app);
    match app.tab {
        1 => render_devices(f, chunks[1], app),
        2 => render_filesystems(f, chunks[1], app),
        3 => render_subvolumes(f, chunks[1], app),
        4 => render_snapshots(f, chunks[1], app),
        5 => render_shares(f, chunks[1], app),
        6 => render_files(f, chunks[1], app),
        7 => render_protocols(f, chunks[1], app),
        8 => render_users(f, chunks[1], app),
        9 => render_alerts(f, chunks[1], app),
        10 => render_system(f, chunks[1], app),
        _ => render_overview(f, chunks[1], app),
    }
    match &app.modal {
        Modal::Form(form) => render_form(f, f.area(), form),
        Modal::Confirm(confirm) => render_confirm(f, f.area(), confirm),
        Modal::Reveal(reveal) => render_reveal(f, f.area(), reveal),
        Modal::Detail(detail) => render_detail(f, f.area(), detail),
        Modal::Logs(logs) => render_logs(f, f.area(), logs, app),
        Modal::FsStatus(fss) => render_fs_status(f, f.area(), fss, app),
        Modal::None => {
            if app.show_help {
                render_help_popup(f, f.area());
            }
        }
    }
}

fn render_fs_status(f: &mut Frame, area: Rect, fss: &super::app::FsStatus, app: &App) {
    let [outer] = Layout::vertical([Constraint::Length(16)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(72)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);
    let title = format!(
        "{} — s scrub · c cancel · f fsck · r refresh · esc close",
        fss.name
    );
    let block = theme::panel(&title).border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let [usage_area, gap, scrub_area] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(1),
        Constraint::Min(4),
    ])
    .areas(inner);

    // Usage: totals + a bar.
    let u = app.fs_usage.clone().unwrap_or(Value::Null);
    let total = u.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    let used = u.get("used_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    let avail = u
        .get("available_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let pct = if total > 0 {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let [u_lines, u_bar] =
        Layout::vertical([Constraint::Length(4), Constraint::Length(1)]).areas(usage_area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            kv("used", &bytes_to_human(used)),
            kv("available", &bytes_to_human(avail)),
            kv("total", &bytes_to_human(total)),
        ]),
        u_lines,
    );
    let bar_color = if pct > 90.0 {
        theme::RED
    } else if pct > 75.0 {
        theme::YELLOW
    } else {
        theme::GREEN
    };
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(bar_color).bg(theme::SURFACE_LO))
            .ratio(pct / 100.0)
            .label(format!("{pct:.0}%")),
        u_bar,
    );
    let _ = gap;

    // Scrub status.
    let sc = app.fs_scrub.clone().unwrap_or(Value::Null);
    let status = field(&sc, "status");
    let color = match status.as_str() {
        "running" => theme::YELLOW,
        "finished" => theme::GREEN,
        "aborted" | "interrupted" => theme::RED,
        _ => theme::SUBTEXT,
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(" scrub", theme::title())),
            Line::from(vec![
                kv_key("status"),
                Span::styled(status, Style::default().fg(color)),
            ]),
            kv("scrubbed", &bytes(sc.get("bytes_scrubbed"))),
            kv("errors", &field(&sc, "error_summary")),
        ]),
        scrub_area,
    );
}

fn render_logs(f: &mut Frame, area: Rect, logs: &super::app::Logs, app: &App) {
    let [outer] = Layout::vertical([Constraint::Percentage(85)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Percentage(88)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);

    let title = format!("logs · {} — j/k scroll · r refresh · esc close", logs.unit);
    let block = theme::panel(&title).border_style(Style::default().fg(theme::ACCENT));
    let text = app.logs.clone().unwrap_or_default();
    let lines: Vec<Line> = text
        .lines()
        .map(|l| {
            let color = if l.contains("ERROR") || l.contains("error") {
                theme::RED
            } else if l.contains("WARN") || l.contains("warn") {
                theme::YELLOW
            } else {
                theme::SUBTEXT
            };
            Line::from(Span::styled(l.to_string(), Style::default().fg(color)))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(block).scroll((logs.scroll, 0)),
        card,
    );
}

fn render_detail(f: &mut Frame, area: Rect, detail: &super::app::Detail) {
    let [outer] = Layout::vertical([Constraint::Percentage(70)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Percentage(70)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);

    let block = theme::panel(&detail.title).border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let [table_area, hint_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(inner);

    if detail.rows.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("(none yet — a to add)", theme::dim()))
                .alignment(Alignment::Center),
            table_area,
        );
    } else {
        let header = Row::new(
            detail
                .headers
                .iter()
                .map(|h| Cell::from(format!(" {h}")))
                .collect::<Vec<_>>(),
        )
        .style(theme::table_header());
        let rows: Vec<Row> = detail
            .rows
            .iter()
            .map(|r| {
                Row::new(
                    r.iter()
                        .map(|c| Cell::from(format!(" {c}")))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let widths = vec![Constraint::Ratio(1, detail.headers.len() as u32); detail.headers.len()];
        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(theme::selected_row())
            .highlight_symbol(Span::styled("▌", Style::default().fg(theme::ACCENT)));
        let mut state = TableState::default();
        state.select(Some(detail.selected));
        f.render_stateful_widget(table, table_area, &mut state);
    }

    f.render_widget(
        Paragraph::new(Span::styled(
            format!("{} · esc close", detail.hint),
            theme::dim(),
        )),
        hint_area,
    );
}

fn render_reveal(f: &mut Frame, area: Rect, reveal: &super::app::Reveal) {
    let [outer] = Layout::vertical([Constraint::Length(8)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(76)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);
    let block = theme::panel(&reveal.title).border_style(Style::default().fg(theme::GREEN));
    let inner = block.inner(card);
    f.render_widget(block, card);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(Span::styled(
            reveal.secret.clone(),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            "this is the only time it is shown — press any key to dismiss",
            theme::dim(),
        ))
        .alignment(Alignment::Center),
        rows[3],
    );
}

// ── header ──────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    // 11 tabs would overflow a narrow terminal if every one showed its
    // label. So: the active tab shows " icon Label " (padded pill), the
    // rest show just their icon. Number keys 1-9/0 still jump directly.
    // Full width = every tab's " icon Label " + dividers + the brand/status
    // on the border. Go compact when that would overflow.
    let full_w: u16 = TABS
        .iter()
        .map(|t| t.len() as u16 + 5) // " icon Label " + divider
        .sum::<u16>()
        + 24;
    let compact = area.width < full_w;

    let titles: Vec<Line> = TABS
        .iter()
        .zip(TAB_ICONS)
        .enumerate()
        .map(|(i, (name, icon))| {
            if compact && i != app.tab {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(icon, Style::default().fg(theme::ACCENT)),
                    Span::raw(" "),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(format!("{icon} "), Style::default().fg(theme::ACCENT)),
                    Span::styled(*name, theme::subtle()),
                    Span::raw(" "),
                ])
            }
        })
        .collect();

    let mut right = vec![if app.connected {
        Span::styled("● ", Style::default().fg(theme::GREEN))
    } else {
        Span::styled("● ", Style::default().fg(theme::RED))
    }];
    right.push(Span::styled(app.status.clone(), theme::dim()));
    if !app.username.is_empty() {
        right.push(Span::styled(
            format!(" · {} · {} ", app.username, app.role),
            theme::dim(),
        ));
    }

    let block = theme::panel_bare()
        .title(Line::from(vec![
            Span::styled(" ⬢ ", Style::default().fg(theme::ACCENT)),
            Span::styled("nastty ", theme::title()),
        ]))
        .title(Line::from(right).right_aligned());

    let tabs = Tabs::new(titles)
        .select(app.tab)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("·", theme::dim()));
    f.render_widget(tabs, area);
}

// ── overview ────────────────────────────────────────────────────

fn render_overview(f: &mut Frame, area: Rect, app: &App) {
    // Active alerts get a banner strip above everything else.
    let (banner_h, body) = if app.alerts.is_empty() {
        (0, area)
    } else {
        let [banner, body] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
        render_alert_banner(f, banner, app);
        (3, body)
    };
    let _ = banner_h;

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);

    render_system_card(f, left, app);

    let [meters, tiles] =
        Layout::vertical([Constraint::Length(12), Constraint::Min(8)]).areas(right);
    render_meters(f, meters, app);
    render_stat_tiles(f, tiles, app);
}

fn render_alert_banner(f: &mut Frame, area: Rect, app: &App) {
    let critical = app
        .alerts
        .iter()
        .any(|a| field(a, "severity") == "critical");
    let color = if critical { theme::RED } else { theme::YELLOW };
    let text = app
        .alerts
        .iter()
        .take(3)
        .map(|a| any(a, &["message", "name"]))
        .collect::<Vec<_>>()
        .join("  ·  ");
    let block = theme::panel_bare().border_style(Style::default().fg(color));
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("⚠ {} alert(s): {text}", app.alerts.len()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .block(block),
        area,
    );
}

/// Live meters: CPU + memory gauges with sparkline history, and one
/// usage gauge per filesystem.
fn render_meters(f: &mut Frame, area: Rect, app: &App) {
    let block = theme::panel("live");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.stats.is_none() {
        let [center] = Layout::vertical([Constraint::Length(2)])
            .flex(Flex::Center)
            .areas(inner);
        f.render_widget(
            Paragraph::new(Span::styled(
                "metrics unavailable — start the nasty-metrics daemon (make metrics)",
                theme::dim(),
            ))
            .alignment(Alignment::Center),
            center,
        );
        return;
    }

    let stats = app.stats.clone().unwrap_or(Value::Null);
    let cores = stats
        .pointer("/cpu/count")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as f64;
    let load1 = stats
        .pointer("/cpu/load_1")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let cpu_pct = ((load1 / cores) * 100.0).clamp(0.0, 100.0);
    let mem_total = stats
        .pointer("/memory/total_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let mem_used = stats
        .pointer("/memory/used_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let mem_pct = (mem_used as f64 / mem_total as f64 * 100.0).clamp(0.0, 100.0);
    let temp = stats
        .pointer("/cpu/temp_c")
        .and_then(|v| v.as_i64())
        .map(|t| format!(" · {t}°C"))
        .unwrap_or_default();

    let mut rows = vec![
        Constraint::Length(1), // cpu gauge
        Constraint::Length(1), // cpu sparkline
        Constraint::Length(1), // mem gauge
        Constraint::Length(1), // mem sparkline
        Constraint::Length(1), // spacer
    ];
    let fs_count = app.filesystems.len().min(4);
    rows.extend(vec![Constraint::Length(1); fs_count]);
    rows.push(Constraint::Min(0));
    let slots = Layout::vertical(rows).split(inner);

    gauge_line(
        f,
        slots[0],
        &format!("cpu  load {load1:.1}/{cores:.0}{temp}"),
        cpu_pct,
        theme::BLUE,
    );
    f.render_widget(
        Sparkline::default()
            .data(&app.cpu_history)
            .max(100)
            .style(Style::default().fg(theme::BLUE)),
        slots[1].inner(ratatui::layout::Margin::new(2, 0)),
    );
    gauge_line(
        f,
        slots[2],
        &format!(
            "mem  {} / {}",
            human_bytes(mem_used),
            human_bytes(mem_total)
        ),
        mem_pct,
        theme::MAUVE,
    );
    f.render_widget(
        Sparkline::default()
            .data(&app.mem_history)
            .max(100)
            .style(Style::default().fg(theme::MAUVE)),
        slots[3].inner(ratatui::layout::Margin::new(2, 0)),
    );

    for (i, fs) in app.filesystems.iter().take(fs_count).enumerate() {
        let total = fs.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
        let used = fs.get("used_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
        let pct = if total > 0 {
            (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let color = if pct > 90.0 {
            theme::RED
        } else if pct > 75.0 {
            theme::YELLOW
        } else {
            theme::GREEN
        };
        gauge_line(
            f,
            slots[5 + i],
            &format!(
                "{}  {} / {}",
                field(fs, "name"),
                human_bytes(used),
                human_bytes(total)
            ),
            pct,
            color,
        );
    }
}

fn gauge_line(f: &mut Frame, area: Rect, label: &str, pct: f64, color: ratatui::style::Color) {
    let [name, bar] = Layout::horizontal([Constraint::Length(30), Constraint::Min(10)]).areas(area);
    f.render_widget(
        Paragraph::new(Span::styled(format!(" {label}"), theme::subtle())),
        name,
    );
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(color).bg(theme::SURFACE_LO))
            .ratio(pct / 100.0)
            .label(Span::styled(
                format!("{pct:.0}%"),
                Style::default().fg(theme::TEXT),
            )),
        bar,
    );
}

fn human_bytes(n: u64) -> String {
    bytes_to_human(n)
}

fn render_system_card(f: &mut Frame, area: Rect, app: &App) {
    let info = app.system_info.clone().unwrap_or(Value::Null);
    let conn = if app.connected {
        Span::styled("● connected", Style::default().fg(theme::GREEN))
    } else {
        Span::styled("● disconnected", Style::default().fg(theme::RED))
    };
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![kv_key("server"), conn]),
        kv("account", &format!("{} ({})", app.username, app.role)),
        Line::from(""),
        kv("hostname", &field(&info, "hostname")),
        kv("kernel", &field(&info, "kernel")),
        kv(
            "uptime",
            &secs_to_human(info.get("uptime_seconds").and_then(|v| v.as_u64())),
        ),
        kv("timezone", &field(&info, "timezone")),
        Line::from(""),
        kv("engine", &field(&info, "version")),
        Line::from(vec![kv_key("btrfs"), backend_span(btrfs_version(&info))]),
        Line::from(vec![
            kv_key("bcachefs"),
            backend_span(bcachefs_version(&info)),
        ]),
        kv("kvm", &field(&info, "kvm_available")),
    ];
    // Only alarm when there is NO usable storage backend at all.
    if btrfs_version(&info).is_none() && bcachefs_version(&info).is_none() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ✗ no storage backend — install btrfs-progs or bcachefs-tools",
            Style::default().fg(theme::RED),
        )));
    }

    f.render_widget(Paragraph::new(lines).block(theme::panel("system")), area);
}

/// bcachefs version when the server reports a usable one.
fn bcachefs_version(info: &Value) -> Option<String> {
    match info.get("bcachefs_version").and_then(|v| v.as_str()) {
        Some(v) if v != "unknown" && !v.is_empty() => Some(v.to_string()),
        _ => None,
    }
}

/// btrfs-progs version injected by nasttyd into system.info.
fn btrfs_version(info: &Value) -> Option<String> {
    info.get("btrfs_version")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

/// A backend line: its version, or a calm dim "not installed".
fn backend_span(version: Option<String>) -> Span<'static> {
    match version {
        Some(v) => Span::styled(format!("● {v}"), Style::default().fg(theme::GREEN)),
        None => Span::styled("○ not installed", theme::dim()),
    }
}

/// True when the server reports a usable bcachefs.
fn bcachefs_available(info: &Value) -> bool {
    bcachefs_version(info).is_some()
}

fn render_stat_tiles(f: &mut Frame, area: Rect, app: &App) {
    let rows =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let top =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);
    let bottom =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);

    let enabled = app
        .protocols
        .iter()
        .filter(|p| p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();

    stat_tile(
        f,
        top[0],
        "devices",
        &app.devices.len().to_string(),
        theme::BLUE,
    );
    stat_tile(
        f,
        top[1],
        "filesystems",
        &app.filesystems.len().to_string(),
        theme::MAUVE,
    );
    stat_tile(
        f,
        bottom[0],
        "shares",
        &(app.nfs.len() + app.smb.len()).to_string(),
        theme::ACCENT,
    );
    stat_tile(
        f,
        bottom[1],
        "protocols on",
        &format!("{enabled}/{}", app.protocols.len()),
        theme::PEACH,
    );
}

/// One stat tile: a large centered number with a dim caption.
fn stat_tile(f: &mut Frame, area: Rect, caption: &str, value: &str, color: ratatui::style::Color) {
    let block = theme::panel_bare();
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [_, num_area, cap_area, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(inner);

    f.render_widget(
        Paragraph::new(Span::styled(
            value.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        num_area,
    );
    f.render_widget(
        Paragraph::new(Span::styled(caption.to_string(), theme::dim()))
            .alignment(Alignment::Center),
        cap_area,
    );
}

// ── two-line card cells ─────────────────────────────────────────

/// A cell with a primary line and a dim secondary line underneath.
/// Both lines get a leading space so card content never touches the
/// selection edge.
fn cell2<'a>(primary: Span<'a>, secondary: Span<'a>) -> Cell<'a> {
    Cell::from(Text::from(vec![
        Line::from(vec![Span::raw(" "), primary]),
        Line::from(vec![Span::raw(" "), secondary]),
    ]))
}

/// A single-line cell, vertically padded to match `cell2` rows.
fn cell1<'a>(content: Span<'a>) -> Cell<'a> {
    Cell::from(Text::from(vec![Line::from(vec![Span::raw(" "), content])]))
}

fn primary(s: String) -> Span<'static> {
    Span::styled(s, theme::text().add_modifier(Modifier::BOLD))
}

fn secondary(s: String) -> Span<'static> {
    Span::styled(s, theme::dim())
}

// ── data tabs ───────────────────────────────────────────────────

fn render_devices(f: &mut Frame, area: Rect, app: &App) {
    // SMART health from system.disks, keyed by device basename (sda…).
    let smart_for = |path: &str| -> Option<&Value> {
        let base = path.rsplit('/').next().unwrap_or(path);
        app.disks
            .iter()
            .find(|d| field(d, "device") == base || field(d, "device") == path)
    };
    let rows: Vec<Row> = app
        .devices
        .iter()
        .map(|d| {
            let class = field(d, "device_class");
            let in_use = d.get("in_use").and_then(|v| v.as_bool()).unwrap_or(false);
            let smart = smart_for(&field(d, "path"));
            let health_cell = match smart {
                Some(s) => {
                    let passed = s.get("health_passed").and_then(|v| v.as_bool());
                    let temp = s
                        .get("temperature_c")
                        .and_then(|v| v.as_i64())
                        .map(|t| format!(" {t}°C"))
                        .unwrap_or_default();
                    match passed {
                        Some(true) => Span::styled(
                            format!("● SMART ok{temp}"),
                            Style::default().fg(theme::GREEN),
                        ),
                        Some(false) => Span::styled(
                            format!("● SMART FAIL{temp}"),
                            Style::default().fg(theme::RED),
                        ),
                        None => Span::styled(format!("SMART n/a{temp}"), theme::dim()),
                    }
                }
                None => Span::styled("—", theme::dim()),
            };
            Row::new(vec![
                cell2(
                    primary(field(d, "path")),
                    secondary(format!("{} · {}", field(d, "model"), field(d, "serial"))),
                ),
                cell2(
                    Span::styled(
                        class.clone(),
                        Style::default().fg(theme::device_class_color(&class)),
                    ),
                    secondary(format!(
                        "{} · {}",
                        field(d, "dev_type"),
                        field(d, "transport")
                    )),
                ),
                cell1(Span::styled(bytes(d.get("size_bytes")), theme::text())),
                cell1(health_cell),
                cell1(if in_use {
                    Span::styled("● in use", Style::default().fg(theme::PEACH))
                } else {
                    Span::styled("○ free", Style::default().fg(theme::GREEN))
                }),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "devices ({}) — ↵ members · w wipe · t type",
            app.devices.len()
        ),
        &["device", "class", "size", "health", "state"],
        &[
            Constraint::Min(26),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(10),
        ],
        rows,
        app.selected,
        "no block devices found",
    );
}

fn render_filesystems(f: &mut Frame, area: Rect, app: &App) {
    // The empty state tells the truth about what this host can create.
    let empty_text = match &app.system_info {
        Some(info) if btrfs_version(info).is_none() && !bcachefs_available(info) => {
            "no storage backend — install btrfs-progs (or bcachefs-tools) first"
        }
        Some(info) if !bcachefs_available(info) => {
            "no filesystems — create one with fs.create (backend: btrfs)"
        }
        _ => "no filesystems — create one with fs.create",
    };
    let rows: Vec<Row> = app
        .filesystems
        .iter()
        .map(|fs| {
            let mounted = fs.get("mounted").and_then(|v| v.as_bool()).unwrap_or(false);
            let backend = field(fs, "backend");
            let backend_color = match backend.as_str() {
                "btrfs" => theme::GREEN,
                "bcachefs" => theme::MAUVE,
                _ => theme::MUTED,
            };
            Row::new(vec![
                cell2(
                    primary(field(fs, "name")),
                    secondary(field(fs, "mount_point")),
                ),
                cell1(Span::styled(backend, Style::default().fg(backend_color))),
                cell1(theme::badge(mounted, "mounted", "unmounted")),
                cell1(Span::styled(field(fs, "state"), theme::subtle())),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "filesystems ({}) — ↵ devices · i status · m mount · s scrub · D destroy",
            app.filesystems.len()
        ),
        &["filesystem", "backend", "status", "state"],
        &[
            Constraint::Min(28),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
        empty_text,
    );
}

fn render_subvolumes(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .subvolumes
        .iter()
        .map(|s| {
            Row::new(vec![
                cell2(
                    primary(field(s, "name")),
                    secondary(format!("on {}", field(s, "filesystem"))),
                ),
                cell1(Span::styled(
                    any(s, &["subvolume_type", "type", "kind"]),
                    Style::default().fg(theme::ACCENT),
                )),
                cell1(Span::styled(
                    bytes(s.get("used_bytes").or_else(|| s.get("size_bytes"))),
                    theme::text(),
                )),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "subvolumes ({}) — n new · c clone · e edit · r resize · s snap · d del",
            app.subvolumes.len()
        ),
        &["subvolume", "type", "used"],
        &[
            Constraint::Min(30),
            Constraint::Length(14),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
        "no subvolumes yet",
    );
}

fn render_shares(f: &mut Frame, area: Rect, app: &App) {
    // 2×2 grid: NFS | SMB / iSCSI | NVMe-oF. One selection runs across
    // all four sections in order; each quadrant highlights its own slice.
    let [top, bottom] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let [nfs_a, smb_a] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top);
    let [iscsi_a, nvme_a] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(bottom);

    let sel = app.selected;
    let nfs_end = app.nfs.len();
    let smb_end = nfs_end + app.smb.len();
    let iscsi_end = smb_end + app.iscsi.len();
    let sel_in = |lo: usize, hi: usize| {
        if sel >= lo && sel < hi {
            sel - lo
        } else {
            usize::MAX
        }
    };

    let nfs_rows: Vec<Row> = app
        .nfs
        .iter()
        .map(|s| {
            let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            Row::new(vec![
                cell2(primary(field(s, "path")), secondary(field(s, "id"))),
                cell1(theme::badge(enabled, "on", "off")),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        nfs_a,
        &format!("nfs ({}) · ↵ clients", app.nfs.len()),
        &["export", "state"],
        &[Constraint::Min(20), Constraint::Length(8)],
        nfs_rows,
        sel_in(0, nfs_end),
        "no NFS shares",
    );

    let smb_rows: Vec<Row> = app
        .smb
        .iter()
        .map(|s| {
            let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            Row::new(vec![
                cell2(primary(any(s, &["name"])), secondary(field(s, "path"))),
                cell1(theme::badge(enabled, "on", "off")),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        smb_a,
        &format!("smb ({}) · ↵ settings", app.smb.len()),
        &["share", "state"],
        &[Constraint::Min(20), Constraint::Length(8)],
        smb_rows,
        sel_in(nfs_end, smb_end),
        "no SMB shares",
    );

    let iscsi_rows: Vec<Row> = app
        .iscsi
        .iter()
        .map(|t| {
            Row::new(vec![
                cell2(primary(field(t, "name")), secondary(field(t, "iqn"))),
                cell1(Span::styled(
                    format!(
                        "{} lun(s)",
                        t.get("luns")
                            .and_then(|v| v.as_array())
                            .map(Vec::len)
                            .unwrap_or(0)
                    ),
                    theme::subtle(),
                )),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        iscsi_a,
        &format!("iscsi ({}) · ↵ luns", app.iscsi.len()),
        &["target", "luns"],
        &[Constraint::Min(20), Constraint::Length(10)],
        iscsi_rows,
        sel_in(smb_end, iscsi_end),
        "no iSCSI targets",
    );

    let nvme_rows: Vec<Row> = app
        .nvmeof
        .iter()
        .map(|s| {
            Row::new(vec![
                cell2(primary(field(s, "name")), secondary(field(s, "nqn"))),
                cell1(Span::styled(
                    format!(
                        "{} ns",
                        s.get("namespaces")
                            .and_then(|v| v.as_array())
                            .map(Vec::len)
                            .unwrap_or(0)
                    ),
                    theme::subtle(),
                )),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        nvme_a,
        &format!("nvme-of ({}) · ↵ ns", app.nvmeof.len()),
        &["subsystem", "ns"],
        &[Constraint::Min(20), Constraint::Length(8)],
        nvme_rows,
        sel_in(iscsi_end, iscsi_end + app.nvmeof.len()),
        "no NVMe-oF subsystems",
    );
}

fn render_files(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .files
        .iter()
        .map(|e| {
            let is_dir = e.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
            let (icon, color) = if is_dir {
                ("🗀", theme::ACCENT)
            } else {
                ("🗎", theme::TEXT)
            };
            let size = if is_dir {
                "—".to_string()
            } else {
                bytes(e.get("size_bytes"))
            };
            Row::new(vec![
                Cell::from(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(format!("{icon} "), Style::default().fg(color)),
                    Span::styled(field(e, "name"), Style::default().fg(color)),
                ])),
                Cell::from(Line::from(Span::styled(size, theme::subtle())).right_aligned()),
            ])
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "{} — ↵ open · ⌫ up · n folder · R rename · d delete",
            app.cwd
        ),
        &["name", "size"],
        &[Constraint::Min(30), Constraint::Length(12)],
        rows,
        app.selected,
        "empty folder",
    );
}

fn render_protocols(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .protocols
        .iter()
        .map(|p| {
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let running = p.get("running").and_then(|v| v.as_bool()).unwrap_or(false);
            Row::new(vec![
                cell2(
                    primary(any(p, &["display_name", "name"])),
                    secondary(field(p, "name")),
                ),
                cell1(theme::badge(enabled, "enabled", "disabled")),
                cell1(theme::badge(running, "running", "stopped")),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        "protocols — enter to toggle",
        &["protocol", "enabled", "service"],
        &[
            Constraint::Min(24),
            Constraint::Length(14),
            Constraint::Length(14),
        ],
        rows,
        app.selected,
        "no protocol data yet",
    );
}

fn render_snapshots(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .snapshots
        .iter()
        .map(|s| {
            Row::new(vec![
                cell2(
                    primary(field(s, "name")),
                    secondary(format!("on {}", field(s, "filesystem"))),
                ),
                cell1(Span::styled(
                    any(s, &["subvolume"]),
                    Style::default().fg(theme::BLUE),
                )),
                cell1(Span::styled(field(s, "backend"), theme::dim())),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "snapshots ({}) — n new · c clone · d delete",
            app.snapshots.len()
        ),
        &["snapshot", "subvolume", "backend"],
        &[
            Constraint::Min(30),
            Constraint::Length(20),
            Constraint::Length(10),
        ],
        rows,
        app.selected,
        "no snapshots — take one from the Subvolumes tab with s",
    );
}

fn render_users(f: &mut Frame, area: Rect, app: &App) {
    // One selectable list spanning three sections; rows carry a section
    // label in the second column so the flat list stays readable.
    let mut rows: Vec<Row> = Vec::new();
    for u in &app.users {
        let role = field(u, "role");
        let must = u
            .get("must_change_password")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        rows.push(
            Row::new(vec![
                cell2(
                    primary(field(u, "username")),
                    secondary(if must {
                        "must change password".to_string()
                    } else {
                        "account".to_string()
                    }),
                ),
                cell1(Span::styled("account", theme::dim())),
                cell1(Span::styled(
                    role.clone(),
                    Style::default().fg(match role.as_str() {
                        "admin" => theme::MAUVE,
                        "operator" => theme::BLUE,
                        _ => theme::SUBTEXT,
                    }),
                )),
            ])
            .height(2),
        );
    }
    for u in &app.smb_users {
        rows.push(
            Row::new(vec![
                cell2(primary(field(u, "username")), secondary("smb user".into())),
                cell1(Span::styled("smb", theme::dim())),
                cell1(Span::styled(
                    any(u, &["groups"]),
                    Style::default().fg(theme::SUBTEXT),
                )),
            ])
            .height(2),
        );
    }
    for g in &app.smb_groups {
        rows.push(
            Row::new(vec![
                cell2(primary(field(g, "name")), secondary("group".into())),
                cell1(Span::styled("group", theme::dim())),
                cell1(Span::styled(
                    any(g, &["members"]),
                    Style::default().fg(theme::SUBTEXT),
                )),
            ])
            .height(2),
        );
    }
    for t in &app.tokens {
        let scope = t
            .get("filesystem")
            .and_then(|v| v.as_str())
            .map(|f| format!("fs:{f}"))
            .unwrap_or_else(|| "all".into());
        let exp = t
            .get("expires_at")
            .and_then(|v| v.as_u64())
            .map(|_| "expires")
            .unwrap_or("never expires");
        rows.push(
            Row::new(vec![
                cell2(primary(field(t, "name")), secondary(exp.into())),
                cell1(Span::styled("token", theme::dim())),
                cell1(Span::styled(
                    format!("{} · {scope}", field(t, "role")),
                    Style::default().fg(theme::SUBTEXT),
                )),
            ])
            .height(2),
        );
    }
    let hint = match app.users_selection() {
        UsersSelection::Account(_) => "n new · d delete · p password",
        UsersSelection::SmbUser(_) => "n new · d delete · p password · g/G groups",
        UsersSelection::Group(_) => "n new · d delete",
        UsersSelection::Token(_) => "n new · d revoke",
    };
    render_table(
        f,
        area,
        &format!(
            "users ({} accounts · {} smb · {} groups · {} tokens) — {hint}",
            app.users.len(),
            app.smb_users.len(),
            app.smb_groups.len(),
            app.tokens.len()
        ),
        &["name", "kind", "detail"],
        &[
            Constraint::Min(24),
            Constraint::Length(10),
            Constraint::Length(24),
        ],
        rows,
        app.selected,
        "no user data (admin required for accounts)",
    );
}

fn render_alerts(f: &mut Frame, area: Rect, app: &App) {
    // Active alerts on top (when any), rules table below.
    let (active_area, rules_area) = if app.alerts.is_empty() {
        (None, area)
    } else {
        let h = (app.alerts.len().min(4) + 2) as u16;
        let [a, r] = Layout::vertical([Constraint::Length(h), Constraint::Min(6)]).areas(area);
        (Some(a), r)
    };
    if let Some(a) = active_area {
        let lines: Vec<Line> = app
            .alerts
            .iter()
            .take(4)
            .map(|al| {
                let sev = field(al, "severity");
                let color = if sev == "critical" {
                    theme::RED
                } else {
                    theme::YELLOW
                };
                Line::from(vec![
                    Span::styled(format!(" ● {sev:<9}"), Style::default().fg(color)),
                    Span::styled(any(al, &["message", "name"]), theme::text()),
                ])
            })
            .collect();
        f.render_widget(
            Paragraph::new(lines)
                .block(theme::panel("active alerts").border_style(Style::default().fg(theme::RED))),
            a,
        );
    }

    let rows: Vec<Row> = app
        .alert_rules
        .iter()
        .map(|r| {
            let enabled = r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let sev = field(r, "severity");
            Row::new(vec![
                cell2(
                    primary(field(r, "name")),
                    secondary(format!(
                        "{} {} {}",
                        field(r, "metric"),
                        field(r, "condition"),
                        field(r, "threshold")
                    )),
                ),
                cell1(Span::styled(
                    sev.clone(),
                    Style::default().fg(if sev == "critical" {
                        theme::RED
                    } else {
                        theme::YELLOW
                    }),
                )),
                cell1(theme::badge(enabled, "enabled", "disabled")),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        rules_area,
        &format!(
            "alert rules ({}) — n new · ↵/e toggle · d delete",
            app.alert_rules.len()
        ),
        &["rule", "severity", "state"],
        &[
            Constraint::Min(34),
            Constraint::Length(12),
            Constraint::Length(14),
        ],
        rows,
        app.selected,
        "no alert rules",
    );
}

fn render_system(f: &mut Frame, area: Rect, app: &App) {
    use super::app::SystemRowKind;
    let rows_data = app.system_rows();
    let rows: Vec<Row> = rows_data
        .iter()
        .map(|r| {
            // Section-header rows (Info with empty value) render as a
            // single dim label spanning the row.
            let is_header = matches!(r.kind, SystemRowKind::Info) && r.value.is_empty();
            if is_header {
                return Row::new(vec![
                    Cell::from(Span::styled(
                        format!(" {}", r.label),
                        theme::title().add_modifier(Modifier::DIM),
                    )),
                    Cell::from(""),
                ])
                .height(1);
            }
            let kind = match r.kind {
                SystemRowKind::Edit { .. } => "edit",
                SystemRowKind::Toggle { .. } => "toggle",
                SystemRowKind::SshKey(_) => "ssh",
                SystemRowKind::TestChannel(_) => "test",
                SystemRowKind::Info => "info",
            };
            Row::new(vec![
                cell2(primary(r.label.clone()), secondary(kind.to_string())),
                cell1(Span::styled(r.value.clone(), theme::subtle())),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        "system — ↵/e edit · n add ssh key · d remove key · L logs",
        &["item", "value"],
        &[Constraint::Min(28), Constraint::Min(24)],
        rows,
        app.selected,
        "no system data yet",
    );
}

// ── modals ──────────────────────────────────────────────────────

fn render_form(f: &mut Frame, area: Rect, form: &Form) {
    let height = (form.fields.len() as u16) + 6;
    let [outer] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(58)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);

    let block = theme::panel(&form.title).border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let mut constraints = vec![Constraint::Length(1)]; // top spacer
    constraints.extend(vec![Constraint::Length(1); form.fields.len()]);
    constraints.push(Constraint::Length(1)); // spacer
    constraints.push(Constraint::Length(1)); // hint
    constraints.push(Constraint::Min(0)); // keys
    let rows = Layout::vertical(constraints).split(inner);

    for (i, fld) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let marker = if focused { "▌ " } else { "  " };
        let value_style = if focused {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::subtle()
        };
        let cursor = if focused && fld.options.is_none() {
            Span::styled("█", Style::default().fg(theme::ACCENT))
        } else {
            Span::raw("")
        };
        let mut line = Paragraph::new(Line::from(vec![
            Span::styled(marker, Style::default().fg(theme::ACCENT)),
            Span::styled(format!("{:<14}", fld.label), theme::label()),
            Span::styled(fld.display(), value_style),
            cursor,
        ]));
        if focused {
            line = line.style(Style::default().bg(theme::SURFACE));
        }
        f.render_widget(line, rows[1 + i]);
    }
    f.render_widget(
        Paragraph::new(Span::styled(form.hint.clone(), theme::dim())).alignment(Alignment::Center),
        rows[form.fields.len() + 2],
    );
    f.render_widget(
        Paragraph::new(Line::from(
            [
                theme::chip("↹", "field"),
                theme::chip("◂▸", "choose"),
                theme::chip("↵", "submit"),
                theme::chip("esc", "cancel"),
            ]
            .concat(),
        ))
        .alignment(Alignment::Center),
        rows[form.fields.len() + 3],
    );
}

fn render_confirm(f: &mut Frame, area: Rect, confirm: &Confirm) {
    let height = if confirm.type_to_confirm.is_some() {
        9
    } else {
        7
    };
    let [outer] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .areas(outer);
    f.render_widget(Clear, card);

    let block = theme::panel("confirm").border_style(Style::default().fg(theme::RED));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Span::styled(confirm.message.clone(), theme::text()))
            .alignment(Alignment::Center),
        rows[1],
    );
    if let Some(expected) = &confirm.type_to_confirm {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("type '{expected}' to confirm: "), theme::dim()),
                Span::styled(
                    confirm.input.clone(),
                    Style::default()
                        .fg(theme::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", Style::default().fg(theme::ACCENT)),
            ]))
            .alignment(Alignment::Center),
            rows[2],
        );
        f.render_widget(
            Paragraph::new(Line::from(
                [theme::chip("↵", "confirm"), theme::chip("esc", "cancel")].concat(),
            ))
            .alignment(Alignment::Center),
            rows[3],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(
                [theme::chip("y/↵", "confirm"), theme::chip("esc", "cancel")].concat(),
            ))
            .alignment(Alignment::Center),
            rows[3],
        );
    }
}

// ── shared table plumbing ───────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn render_table(
    f: &mut Frame,
    area: Rect,
    title: &str,
    headers: &[&str],
    widths: &[Constraint],
    rows: Vec<Row>,
    selected: usize,
    empty_text: &str,
) {
    let empty = rows.is_empty();
    let block = theme::panel(title);

    if empty {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [center] = Layout::vertical([Constraint::Length(1)])
            .flex(ratatui::layout::Flex::Center)
            .areas(inner);
        f.render_widget(
            Paragraph::new(Span::styled(empty_text.to_string(), theme::dim()))
                .alignment(Alignment::Center),
            center,
        );
        return;
    }

    // Header labels get the same leading space as card content so
    // columns line up.
    let header = Row::new(
        headers
            .iter()
            .map(|h| Cell::from(format!(" {h}")))
            .collect::<Vec<_>>(),
    )
    .style(theme::table_header())
    .bottom_margin(1);
    let table = Table::new(rows, widths.to_vec())
        .header(header)
        .block(block)
        .column_spacing(2)
        .row_highlight_style(theme::selected_row())
        .highlight_symbol(Text::from(vec![
            Line::from(Span::styled("▌", Style::default().fg(theme::ACCENT))),
            Line::from(Span::styled("▌", Style::default().fg(theme::ACCENT))),
        ]));

    if selected == usize::MAX {
        f.render_widget(table, area);
    } else {
        let mut state = TableState::default();
        state.select(Some(selected));
        f.render_stateful_widget(table, area, &mut state);
    }
}

fn render_help_popup(f: &mut Frame, area: Rect) {
    let [outer] = Layout::vertical([Constraint::Length(15)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .areas(outer);

    f.render_widget(Clear, popup);

    let lines = vec![
        shortcut("?", "show / close this help"),
        shortcut("1-6", "jump to tab"),
        shortcut("tab / shift-tab", "next / previous tab"),
        shortcut("← →  h l", "switch tabs"),
        shortcut("↑ ↓  k j", "move selection"),
        shortcut("r", "refresh all data"),
        shortcut("enter", "toggle selected protocol on Protocols tab"),
        shortcut("q", "quit"),
        shortcut("ctrl-c", "quit immediately"),
        shortcut("esc", "close help, or quit when help is closed"),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(theme::panel("shortcuts"))
            .alignment(Alignment::Left),
        popup,
    );
}

fn shortcut<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{key:<17}"),
            theme::label().add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, theme::text()),
    ])
}

// ── value helpers ───────────────────────────────────────────────

fn kv_key(key: &str) -> Span<'_> {
    Span::styled(format!("{key:<11}"), theme::label())
}

fn kv<'a>(key: &'a str, val: &str) -> Line<'a> {
    Line::from(vec![
        kv_key(key),
        Span::styled(val.to_string(), theme::text()),
    ])
}

/// Display a single field, coercing common JSON scalar types to text.
fn field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "yes" } else { "no" }.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Null) | None => "-".to_string(),
        Some(other) => other.to_string(),
    }
}

/// First present, non-empty field from a list of candidate keys.
fn any(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(val) = v.get(k)
            && !val.is_null()
        {
            return field(v, k);
        }
    }
    "-".to_string()
}

fn bytes(v: Option<&Value>) -> String {
    match v.and_then(|v| v.as_u64()) {
        Some(n) => bytes_to_human(n),
        None => "-".to_string(),
    }
}

fn bytes_to_human(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut val = n as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}

fn secs_to_human(secs: Option<u64>) -> String {
    let Some(mut s) = secs else {
        return "-".to_string();
    };
    let d = s / 86400;
    s %= 86400;
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_human_scales() {
        assert_eq!(bytes_to_human(512), "512 B");
        assert_eq!(bytes_to_human(1024), "1.0 KiB");
        assert_eq!(bytes_to_human(1536), "1.5 KiB");
        assert_eq!(bytes_to_human(4096805658624), "3.7 TiB");
    }

    #[test]
    fn secs_to_human_formats() {
        assert_eq!(secs_to_human(None), "-");
        assert_eq!(secs_to_human(Some(90)), "1m");
        assert_eq!(secs_to_human(Some(3720)), "1h 2m");
        assert_eq!(secs_to_human(Some(90000)), "1d 1h 0m");
    }

    #[test]
    fn field_coerces_types() {
        let v = serde_json::json!({"s":"x","b":true,"n":5,"nil":null});
        assert_eq!(field(&v, "s"), "x");
        assert_eq!(field(&v, "b"), "yes");
        assert_eq!(field(&v, "n"), "5");
        assert_eq!(field(&v, "nil"), "-");
        assert_eq!(field(&v, "missing"), "-");
    }

    fn buffer_text(app: &App, w: u16, h: u16) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render_app(f, app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn renders_protocols_tab_with_data() {
        let mut app = App::for_test();
        app.tab = 7;
        app.protocols = vec![
            serde_json::json!({"name":"nfs","display_name":"NFS","enabled":true,"running":true}),
            serde_json::json!({"name":"smb","display_name":"SMB","enabled":false,"running":false}),
        ];
        let text = buffer_text(&app, 100, 20);
        assert!(text.contains("protocols"), "tab title missing");
        assert!(text.contains("NFS"), "protocol row missing");
        assert!(text.contains("enabled"), "enabled badge missing");
        assert!(!text.contains("shortcuts"), "help popup should be hidden");
    }

    #[test]
    fn renders_help_popup_when_requested() {
        let mut app = App::for_test();
        app.show_help = true;
        let text = buffer_text(&app, 100, 24);
        assert!(text.contains("shortcuts"), "help title missing");
        assert!(
            text.contains("refresh all data"),
            "refresh shortcut missing"
        );
        assert!(text.contains("quit"), "quit shortcut missing");
    }

    #[test]
    fn renders_overview_with_system_info() {
        let mut app = App::for_test();
        app.username = "admin".to_string();
        app.role = "admin".to_string();
        app.system_info = Some(serde_json::json!({
            "hostname": "tron",
            "kernel": "7.0.0-generic",
            "uptime_seconds": 3720,
            "version": "0.1.0",
        }));
        let text = buffer_text(&app, 100, 24);
        assert!(text.contains("system"), "system card missing");
        assert!(text.contains("tron"), "hostname missing");
        assert!(text.contains("connected"), "connection badge missing");
        assert!(text.contains("devices"), "stat tile caption missing");
    }

    #[test]
    fn renders_empty_state_message() {
        let mut app = App::for_test();
        app.tab = 2; // filesystems, empty
        let text = buffer_text(&app, 100, 20);
        assert!(
            text.contains("no filesystems"),
            "empty-state message missing"
        );
    }

    #[test]
    fn two_line_rows_show_secondary_info() {
        let mut app = App::for_test();
        app.tab = 1;
        app.devices = vec![serde_json::json!({
            "path":"/dev/sda","device_class":"ssd","dev_type":"disk",
            "transport":"sata","size_bytes":1024u64,
            "model":"AcmeDisk","serial":"SN123","in_use":false
        })];
        let text = buffer_text(&app, 110, 20);
        assert!(text.contains("/dev/sda"), "primary line missing");
        assert!(text.contains("AcmeDisk"), "secondary line missing");
        assert!(text.contains("SN123"), "serial missing from secondary");
    }

    /// Visual preview of each tab, for development. Run with:
    /// `cargo test --lib -- --ignored preview --nocapture`
    #[test]
    #[ignore]
    fn preview() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::for_test();
        app.username = "admin".to_string();
        app.role = "admin".to_string();
        app.status = "ready".to_string();
        app.system_info = Some(serde_json::json!({
            "hostname": "tron", "kernel": "7.0.0-27-generic",
            "uptime_seconds": 11520, "timezone": "UTC",
            "version": "0.0.13", "bcachefs_version": "unknown",
            "btrfs_version": "6.17.1", "kvm_available": true,
        }));
        app.devices = vec![
            serde_json::json!({"path":"/dev/sda","device_class":"ssd","dev_type":"disk",
                "transport":"sata","size_bytes":4096805658624u64,
                "model":"TS4TSSD230S","serial":"I216000111","in_use":false}),
            serde_json::json!({"path":"/dev/nvme0n1","device_class":"nvme","dev_type":"disk",
                "transport":"nvme","size_bytes":2048408248320u64,
                "model":"Samsung 990 PRO","serial":"S6Z1NJ0T","in_use":true}),
        ];
        app.protocols = vec![
            serde_json::json!({"name":"nfs","display_name":"NFS","enabled":true,"running":true}),
            serde_json::json!({"name":"smb","display_name":"SMB","enabled":false,"running":false}),
            serde_json::json!({"name":"ssh","display_name":"SSH","enabled":true,"running":true}),
        ];
        app.users = vec![
            serde_json::json!({"username":"admin","role":"admin","must_change_password":false}),
            serde_json::json!({"username":"kush","role":"operator","must_change_password":true}),
        ];
        app.smb_users = vec![serde_json::json!({"username":"media"})];
        app.smb_groups = vec![serde_json::json!({"name":"family"})];
        app.snapshots = vec![serde_json::json!({
            "name":"data@daily","filesystem":"tank","subvolume":"data","backend":"btrfs"
        })];
        app.stats = Some(serde_json::json!({
            "cpu": {"count": 24, "load_1": 3.6, "load_5": 2.0, "load_15": 1.2, "temp_c": 44},
            "memory": {"total_bytes": 67108864000u64, "used_bytes": 21474836480u64},
        }));
        app.cpu_history = vec![5, 8, 12, 20, 15, 30, 25, 40, 35, 20, 18, 22, 30, 15];
        app.mem_history = vec![30, 31, 32, 32, 33, 31, 30, 32, 34, 33, 32, 31, 32, 32];
        app.filesystems = vec![serde_json::json!({
            "name":"tank","backend":"btrfs","mounted":true,"mount_point":"/fs/tank",
            "total_bytes":4000000000000u64,"used_bytes":3350000000000u64
        })];
        app.alerts = vec![serde_json::json!({
            "severity":"warning","message":"Filesystem tank at 84% capacity","name":"fs-usage-warn"
        })];
        app.alert_rules = vec![
            serde_json::json!({"id":"r1","name":"Filesystem usage warning","metric":"fs_usage_percent",
                "condition":"above","threshold":80.0,"severity":"warning","enabled":true}),
            serde_json::json!({"id":"r2","name":"Disk temperature critical","metric":"disk_temperature",
                "condition":"above","threshold":60.0,"severity":"critical","enabled":false}),
        ];
        app.settings = Some(serde_json::json!({
            "hostname":"tron","timezone":"UTC","clock_24h":true,"temp_unit":"celsius"
        }));
        app.ssh = Some(serde_json::json!({
            "password_auth": false,
            "keys": ["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF7 kush@tron"]
        }));
        app.iscsi = vec![serde_json::json!({
            "id":"t1","name":"backupdisk","iqn":"iqn.2137-04.storage.nasty:backupdisk","luns":[{}]
        })];
        app.selected = 1;

        for tab in [0usize, 1, 4, 5, 6, 7, 8, 9, 10] {
            app.tab = tab;
            let mut terminal = Terminal::new(TestBackend::new(110, 26)).unwrap();
            terminal.draw(|f| render_app(f, &app)).unwrap();
            let buf = terminal.backend().buffer().clone();
            println!("── tab {tab} ({}) ──", TABS[tab]);
            for y in 0..buf.area.height {
                let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                println!("{line}");
            }
            println!();
        }
    }
}
