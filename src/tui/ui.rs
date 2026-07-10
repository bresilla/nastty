//! Rendering for the main view. Pure functions over `App` state, so they
//! can be exercised with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs};
use serde_json::Value;

use super::app::{App, TABS};

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

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let who = if app.username.is_empty() {
        " nastty ".to_string()
    } else {
        format!(" nastty — {}@nas ({}) ", app.username, app.role)
    };
    let tabs = Tabs::new(TABS.to_vec())
        .select(app.tab)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(who)
                .title_alignment(Alignment::Center),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" ");
    f.render_widget(tabs, area);
}

fn render_overview(f: &mut Frame, area: Rect, app: &App) {
    let info = app.system_info.clone().unwrap_or(Value::Null);
    let conn = if app.connected {
        Span::styled("● connected", Style::default().fg(Color::Green))
    } else {
        Span::styled("● disconnected", Style::default().fg(Color::Red))
    };
    let lines = vec![
        Line::from(vec![Span::styled("  Server:        ", label()), conn]),
        kv("Signed in as", &format!("{} ({})", app.username, app.role)),
        Line::from(""),
        kv("Hostname", &field(&info, "hostname")),
        kv("Kernel", &field(&info, "kernel")),
        kv(
            "Uptime",
            &secs_to_human(info.get("uptime_seconds").and_then(|v| v.as_u64())),
        ),
        kv("Timezone", &field(&info, "timezone")),
        kv("Engine version", &field(&info, "version")),
        kv("bcachefs", &field(&info, "bcachefs_version")),
        kv("KVM available", &field(&info, "kvm_available")),
    ];

    let block = Block::default().borders(Borders::ALL).title(" Overview ");
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_devices(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .devices
        .iter()
        .map(|d| {
            Row::new(vec![
                field(d, "path"),
                field(d, "dev_type"),
                bytes(d.get("size_bytes")),
                field(d, "model"),
                field(d, "in_use"),
            ])
        })
        .collect();
    render_table(
        f,
        area,
        " Devices ",
        &["Device", "Type", "Size", "Model", "In use"],
        &[
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Min(12),
            Constraint::Length(8),
        ],
        rows,
        app.selected,
    );
}

fn render_filesystems(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .filesystems
        .iter()
        .map(|fs| {
            Row::new(vec![
                field(fs, "name"),
                field(fs, "mounted"),
                field(fs, "mount_point"),
                field(fs, "state"),
            ])
        })
        .collect();
    render_table(
        f,
        area,
        " Filesystems ",
        &["Name", "Mounted", "Mount point", "State"],
        &[
            Constraint::Min(12),
            Constraint::Length(9),
            Constraint::Length(20),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
    );
}

fn render_subvolumes(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .subvolumes
        .iter()
        .map(|s| {
            Row::new(vec![
                field(s, "filesystem"),
                field(s, "name"),
                any(s, &["subvolume_type", "type", "kind"]),
                bytes(s.get("used_bytes").or_else(|| s.get("size_bytes"))),
            ])
        })
        .collect();
    render_table(
        f,
        area,
        " Subvolumes ",
        &["Filesystem", "Name", "Type", "Used"],
        &[
            Constraint::Length(16),
            Constraint::Min(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
        rows,
        app.selected,
    );
}

fn render_shares(f: &mut Frame, area: Rect, app: &App) {
    let halves =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let nfs_rows: Vec<Row> = app
        .nfs
        .iter()
        .map(|s| Row::new(vec![field(s, "id"), field(s, "path")]))
        .collect();
    render_table(
        f,
        halves[0],
        " NFS shares ",
        &["ID", "Path"],
        &[Constraint::Length(20), Constraint::Min(20)],
        nfs_rows,
        usize::MAX, // no selection on shares view
    );

    let smb_rows: Vec<Row> = app
        .smb
        .iter()
        .map(|s| Row::new(vec![field(s, "id"), any(s, &["name"]), field(s, "path")]))
        .collect();
    render_table(
        f,
        halves[1],
        " SMB shares ",
        &["ID", "Name", "Path"],
        &[
            Constraint::Length(20),
            Constraint::Length(16),
            Constraint::Min(20),
        ],
        smb_rows,
        usize::MAX,
    );
}

fn render_protocols(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .protocols
        .iter()
        .map(|p| {
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            let running = p.get("running").and_then(|v| v.as_bool()).unwrap_or(false);
            let enabled_cell = Cell::from(if enabled { "enabled" } else { "disabled" }).style(
                Style::default().fg(if enabled {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            );
            Row::new(vec![
                Cell::from(any(p, &["display_name", "name"])),
                Cell::from(field(p, "name")),
                enabled_cell,
                Cell::from(if running { "yes" } else { "no" }),
            ])
        })
        .collect();
    render_table(
        f,
        area,
        " Protocols  (Enter to toggle) ",
        &["Protocol", "Key", "Enabled", "Running"],
        &[
            Constraint::Min(14),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
        rows,
        app.selected,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_table(
    f: &mut Frame,
    area: Rect,
    title: &str,
    headers: &[&str],
    widths: &[Constraint],
    rows: Vec<Row>,
    selected: usize,
) {
    let header = Row::new(headers.iter().map(|h| Cell::from(*h)).collect::<Vec<_>>()).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let empty = rows.is_empty();
    let table = Table::new(rows, widths.to_vec())
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title.to_string()),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    if selected == usize::MAX {
        f.render_widget(table, area);
    } else {
        let mut state = TableState::default();
        if !empty {
            state.select(Some(selected));
        }
        f.render_stateful_widget(table, area, &mut state);
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let cols =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
    let help = "1-6 tabs · ←/→ · ↑/↓ move · Enter toggle · r refresh · q quit";
    f.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        cols[0],
    );
    let status_style = if app.connected {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    };
    f.render_widget(
        Paragraph::new(app.status.clone())
            .alignment(Alignment::Right)
            .style(status_style),
        cols[1],
    );
}

// ── value helpers ───────────────────────────────────────────────

fn label() -> Style {
    Style::default().fg(Color::Cyan)
}

fn kv<'a>(key: &str, val: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {:<14} ", format!("{key}:")), label()),
        Span::raw(val.to_string()),
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
        assert!(text.contains("Protocols"), "tab title missing");
        assert!(text.contains("NFS"), "protocol row missing");
        assert!(text.contains("enabled"), "enabled cell missing");
        assert!(text.contains("q quit"), "footer help missing");
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
        let text = buffer_text(&app, 100, 20);
        assert!(text.contains("Overview"));
        assert!(text.contains("tron"), "hostname missing");
        assert!(text.contains("connected"), "connection line missing");
    }
}
