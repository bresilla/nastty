//! Rendering for the main view. Pure functions over `App` state, so they
//! can be exercised with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState, Tabs};
use serde_json::Value;

use super::app::{App, TABS};
use super::theme;

const TAB_ICONS: [&str; 6] = ["⌂", "⛁", "▤", "▦", "⇄", "☰"];

pub(super) fn render_app(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(f.area());

    render_tabs(f, chunks[0], app);
    match app.tab {
        1 => render_devices(f, chunks[1], app),
        2 => render_filesystems(f, chunks[1], app),
        3 => render_subvolumes(f, chunks[1], app),
        4 => render_shares(f, chunks[1], app),
        5 => render_protocols(f, chunks[1], app),
        _ => render_overview(f, chunks[1], app),
    }
    if app.show_help {
        render_help_popup(f, f.area());
    }
}

// ── header ──────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    // Each tab is padded inside so the selection pill has air around the
    // icon and label: " ⌂ Overview ".
    let titles: Vec<Line> = TABS
        .iter()
        .zip(TAB_ICONS)
        .map(|(name, icon)| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{icon} "), Style::default().fg(theme::ACCENT)),
                Span::styled(*name, theme::subtle()),
                Span::raw(" "),
            ])
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
        Line::from(vec![kv_key("bcachefs"), bcachefs_span(&info)]),
        kv("kvm", &field(&info, "kvm_available")),
    ];

    f.render_widget(Paragraph::new(lines).block(theme::panel("system")), area);
}

/// True when the server reports a usable bcachefs (kernel module probed).
fn bcachefs_available(info: &Value) -> bool {
    matches!(
        info.get("bcachefs_version").and_then(|v| v.as_str()),
        Some(v) if v != "unknown" && !v.is_empty()
    )
}

fn bcachefs_span(info: &Value) -> Span<'static> {
    if bcachefs_available(info) {
        Span::styled(field(info, "bcachefs_version"), theme::text())
    } else {
        Span::styled(
            "✗ not available — install bcachefs-tools + kernel module",
            Style::default().fg(theme::RED),
        )
    }
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
    let rows: Vec<Row> = app
        .devices
        .iter()
        .map(|d| {
            let class = field(d, "device_class");
            let in_use = d.get("in_use").and_then(|v| v.as_bool()).unwrap_or(false);
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
        &format!("devices ({})", app.devices.len()),
        &["device", "class", "size", "state"],
        &[
            Constraint::Min(30),
            Constraint::Length(18),
            Constraint::Length(11),
            Constraint::Length(11),
        ],
        rows,
        app.selected,
        "no block devices found",
    );
}

fn render_filesystems(f: &mut Frame, area: Rect, app: &App) {
    // The empty state tells the truth about what this host can create.
    let empty_text = match &app.system_info {
        Some(info) if !bcachefs_available(info) => {
            "no filesystems — create one with fs.create (btrfs works on this host; bcachefs needs tools + kernel module)"
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
        &format!("filesystems ({})", app.filesystems.len()),
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
        &format!("subvolumes ({})", app.subvolumes.len()),
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
    let halves =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let nfs_rows: Vec<Row> = app
        .nfs
        .iter()
        .map(|s| {
            Row::new(vec![cell2(
                primary(field(s, "path")),
                secondary(field(s, "id")),
            )])
            .height(2)
        })
        .collect();
    render_table(
        f,
        halves[0],
        &format!("nfs shares ({})", app.nfs.len()),
        &["export"],
        &[Constraint::Min(30)],
        nfs_rows,
        usize::MAX, // no selection on shares view
        "no NFS shares",
    );

    let smb_rows: Vec<Row> = app
        .smb
        .iter()
        .map(|s| {
            Row::new(vec![
                cell2(primary(any(s, &["name"])), secondary(field(s, "id"))),
                cell1(Span::styled(field(s, "path"), theme::subtle())),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        halves[1],
        &format!("smb shares ({})", app.smb.len()),
        &["share", "path"],
        &[Constraint::Length(28), Constraint::Min(24)],
        smb_rows,
        usize::MAX,
        "no SMB shares",
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
        app.tab = 5;
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
            "version": "0.0.13", "bcachefs_version": "1.38.8", "kvm_available": true,
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
        app.selected = 1;

        for tab in [1usize, 5] {
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
