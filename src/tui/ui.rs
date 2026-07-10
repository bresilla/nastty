//! Rendering for the main view. Pure functions over `App` state, so they
//! can be exercised with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState, Tabs};
use serde_json::Value;

use super::app::{App, TABS};
use super::theme;

const TAB_ICONS: [&str; 6] = ["⌂", "⛁", "▤", "▦", "⇄", "☰"];

pub(super) fn render_app(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(f.area());

    render_tabs(f, chunks[0], app);
    match app.tab {
        1 => render_devices(f, chunks[1], app),
        2 => render_filesystems(f, chunks[1], app),
        3 => render_subvolumes(f, chunks[1], app),
        4 => render_shares(f, chunks[1], app),
        5 => render_protocols(f, chunks[1], app),
        _ => render_overview(f, chunks[1], app),
    }
    render_footer(f, chunks[2], app);
}

// ── header ──────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = TABS
        .iter()
        .zip(TAB_ICONS)
        .map(|(name, icon)| {
            Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(theme::TEAL)),
                Span::styled(*name, theme::subtle()),
            ])
        })
        .collect();

    let who = if app.username.is_empty() {
        String::new()
    } else {
        format!(" {} · {} ", app.username, app.role)
    };

    let block = theme::panel_bare()
        .title(Line::from(vec![
            Span::styled(" ⬢ ", Style::default().fg(theme::ACCENT)),
            Span::styled("nastty ", theme::title()),
        ]))
        .title(Line::from(Span::styled(who, theme::dim())).right_aligned());

    let tabs = Tabs::new(titles)
        .select(app.tab)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(theme::SURFACE_LO)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .divider(Span::styled("·", theme::dim()));
    f.render_widget(tabs, area);
}

// ── overview ────────────────────────────────────────────────────

fn render_overview(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);

    render_system_card(f, left, app);
    render_stat_tiles(f, right, app);
}

fn render_system_card(f: &mut Frame, area: Rect, app: &App) {
    let info = app.system_info.clone().unwrap_or(Value::Null);
    let conn = if app.connected {
        Span::styled("● connected", Style::default().fg(theme::GREEN))
    } else {
        Span::styled("● disconnected", Style::default().fg(theme::RED))
    };
    let lines = vec![
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
        kv("bcachefs", &field(&info, "bcachefs_version")),
        kv("kvm", &field(&info, "kvm_available")),
    ];

    f.render_widget(Paragraph::new(lines).block(theme::panel("system")), area);
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
        theme::TEAL,
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

// ── data tabs ───────────────────────────────────────────────────

fn render_devices(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let class = field(d, "device_class");
            let in_use = d.get("in_use").and_then(|v| v.as_bool()).unwrap_or(false);
            Row::new(vec![
                Cell::from(Span::styled(field(d, "path"), theme::text())),
                Cell::from(Span::styled(
                    class.clone(),
                    Style::default().fg(theme::device_class_color(&class)),
                )),
                Cell::from(Span::styled(field(d, "dev_type"), theme::subtle())),
                Cell::from(
                    Line::from(Span::styled(bytes(d.get("size_bytes")), theme::text()))
                        .right_aligned(),
                ),
                Cell::from(Span::styled(field(d, "model"), theme::subtle())),
                Cell::from(Line::from(if in_use {
                    Span::styled("● in use", Style::default().fg(theme::PEACH))
                } else {
                    Span::styled("○ free", Style::default().fg(theme::GREEN))
                })),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        area,
        &format!("devices ({})", app.devices.len()),
        &["device", "class", "type", "size", "model", "state"],
        &[
            Constraint::Length(18),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Min(14),
            Constraint::Length(10),
        ],
        rows,
        app.selected,
        "no block devices found",
    );
}

fn render_filesystems(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .filesystems
        .iter()
        .enumerate()
        .map(|(i, fs)| {
            let mounted = fs.get("mounted").and_then(|v| v.as_bool()).unwrap_or(false);
            Row::new(vec![
                Cell::from(Span::styled(field(fs, "name"), theme::text())),
                Cell::from(Line::from(theme::badge(mounted, "mounted", "unmounted"))),
                Cell::from(Span::styled(field(fs, "mount_point"), theme::subtle())),
                Cell::from(Span::styled(field(fs, "state"), theme::subtle())),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        area,
        &format!("filesystems ({})", app.filesystems.len()),
        &["name", "status", "mount point", "state"],
        &[
            Constraint::Min(14),
            Constraint::Length(12),
            Constraint::Length(24),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
        "no filesystems — create one with fs.create",
    );
}

fn render_subvolumes(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .subvolumes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Row::new(vec![
                Cell::from(Span::styled(field(s, "filesystem"), theme::subtle())),
                Cell::from(Span::styled(field(s, "name"), theme::text())),
                Cell::from(Span::styled(
                    any(s, &["subvolume_type", "type", "kind"]),
                    Style::default().fg(theme::TEAL),
                )),
                Cell::from(
                    Line::from(Span::styled(
                        bytes(s.get("used_bytes").or_else(|| s.get("size_bytes"))),
                        theme::text(),
                    ))
                    .right_aligned(),
                ),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        area,
        &format!("subvolumes ({})", app.subvolumes.len()),
        &["filesystem", "name", "type", "used"],
        &[
            Constraint::Length(16),
            Constraint::Min(14),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
        "no subvolumes yet",
    );
}

fn render_shares(f: &mut Frame, area: Rect, app: &App) {
    let halves =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let nfs_rows: Vec<Row> = app
        .nfs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Row::new(vec![
                Cell::from(Span::styled(field(s, "id"), theme::subtle())),
                Cell::from(Span::styled(field(s, "path"), theme::text())),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        halves[0],
        &format!("nfs shares ({})", app.nfs.len()),
        &["id", "path"],
        &[Constraint::Length(24), Constraint::Min(20)],
        nfs_rows,
        usize::MAX, // no selection on shares view
        "no NFS shares",
    );

    let smb_rows: Vec<Row> = app
        .smb
        .iter()
        .enumerate()
        .map(|(i, s)| {
            Row::new(vec![
                Cell::from(Span::styled(field(s, "id"), theme::subtle())),
                Cell::from(Span::styled(any(s, &["name"]), theme::text())),
                Cell::from(Span::styled(field(s, "path"), theme::text())),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        halves[1],
        &format!("smb shares ({})", app.smb.len()),
        &["id", "name", "path"],
        &[
            Constraint::Length(24),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
        smb_rows,
        usize::MAX,
        "no SMB shares",
    );
}

fn render_protocols(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .protocols
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let running = p.get("running").and_then(|v| v.as_bool()).unwrap_or(false);
            Row::new(vec![
                Cell::from(Span::styled(
                    any(p, &["display_name", "name"]),
                    theme::text(),
                )),
                Cell::from(Span::styled(field(p, "name"), theme::dim())),
                Cell::from(Line::from(theme::badge(enabled, "enabled", "disabled"))),
                Cell::from(Line::from(theme::badge(running, "running", "stopped"))),
            ])
            .style(theme::zebra(i))
        })
        .collect();
    render_table(
        f,
        area,
        "protocols — enter to toggle",
        &["protocol", "key", "enabled", "service"],
        &[
            Constraint::Min(16),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
        "no protocol data yet",
    );
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

    let header = Row::new(headers.iter().map(|h| Cell::from(*h)).collect::<Vec<_>>())
        .style(theme::table_header())
        .bottom_margin(1);
    let table = Table::new(rows, widths.to_vec())
        .header(header)
        .block(block)
        .column_spacing(2)
        .row_highlight_style(theme::selected_row())
        .highlight_symbol(Span::styled("▌ ", Style::default().fg(theme::ACCENT)));

    if selected == usize::MAX {
        f.render_widget(table, area);
    } else {
        let mut state = TableState::default();
        state.select(Some(selected));
        f.render_stateful_widget(table, area, &mut state);
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Length(40)]).split(area);

    let hints = [
        theme::chip("1-6", "tabs"),
        theme::chip("↑↓", "move"),
        theme::chip("↵", "toggle"),
        theme::chip("r", "refresh"),
        theme::chip("q", "quit"),
    ]
    .concat();
    f.render_widget(Paragraph::new(Line::from(hints)), cols[0]);

    let dot = if app.connected {
        Span::styled("● ", Style::default().fg(theme::GREEN))
    } else {
        Span::styled("● ", Style::default().fg(theme::RED))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            dot,
            Span::styled(app.status.clone(), theme::dim()),
        ]))
        .alignment(Alignment::Right),
        cols[1],
    );
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
        app.tab = 5;
        app.protocols = vec![
            serde_json::json!({"name":"nfs","display_name":"NFS","enabled":true,"running":true}),
            serde_json::json!({"name":"smb","display_name":"SMB","enabled":false,"running":false}),
        ];
        let text = buffer_text(&app, 100, 20);
        assert!(text.contains("protocols"), "tab title missing");
        assert!(text.contains("NFS"), "protocol row missing");
        assert!(text.contains("enabled"), "enabled badge missing");
        assert!(text.contains("quit"), "footer help missing");
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
            "version": "0.0.13", "bcachefs_version": "1.38.8", "kvm_available": true,
        }));
        app.devices = vec![
            serde_json::json!({"path":"/dev/sda","device_class":"ssd","dev_type":"disk",
                "size_bytes":4096805658624u64,"model":"TS4TSSD230S","in_use":false}),
            serde_json::json!({"path":"/dev/nvme0n1","device_class":"nvme","dev_type":"disk",
                "size_bytes":2048408248320u64,"model":"Samsung 990 PRO","in_use":true}),
        ];
        app.protocols = vec![
            serde_json::json!({"name":"nfs","display_name":"NFS","enabled":true,"running":true}),
            serde_json::json!({"name":"smb","display_name":"SMB","enabled":false,"running":false}),
            serde_json::json!({"name":"ssh","display_name":"SSH","enabled":true,"running":true}),
        ];

        for tab in [0usize, 1, 5] {
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
