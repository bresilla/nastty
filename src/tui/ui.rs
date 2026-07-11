//! Rendering for the main view. Pure functions over `App` state, so they
//! can be exercised with ratatui's `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Cell, Clear, Gauge, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Sparkline,
    Table, TableState,
};
use ratatui_cheese::paginator::{Paginator, PaginatorMode, PaginatorState};
use ratatui_cheese::spinner::{Spinner, SpinnerState, SpinnerType};
use serde_json::Value;
use tui_overlay::{Anchor, Overlay, OverlayState};
use tui_popup::{KnownSizeWrapper, Popup};
use tui_tabs::TabNav;

use super::app::{
    App, Confirm, Form, GROUPS, GROUPS_COMPACT, Modal, TAB_ICONS, TABS, UsersSelection,
    group_for_tab, group_views,
};
use super::theme;

pub(super) fn render_app(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(f.area());

    render_tabs(f, chunks[0], app);
    let body = chunks[1];
    if body.width >= 90 {
        let [sidebar, workspace] = Layout::horizontal([Constraint::Length(22), Constraint::Min(0)])
            .spacing(1)
            .areas(body);
        render_sidebar(f, sidebar, app);
        render_tab(f, workspace, app);
    } else {
        render_tab(f, body, app);
    }
    if app.inspector_open && app.tab != 0 {
        let mut drawer_state = OverlayState::new().with_duration(std::time::Duration::ZERO);
        drawer_state.open();
        drawer_state.tick(std::time::Duration::from_secs(1));
        let drawer = Overlay::new()
            .anchor(Anchor::Right)
            .width(if body.width >= 120 {
                Constraint::Percentage(38)
            } else {
                Constraint::Percentage(55)
            })
            .height(Constraint::Percentage(100))
            .block(
                theme::panel(inspector_title(app)).border_style(Style::default().fg(theme::MAUVE)),
            );
        f.render_stateful_widget(drawer, body, &mut drawer_state);
        if let Some(inner) = drawer_state.inner_area() {
            render_inspector_content(f, inner, app);
        }
    }
    render_footer(f, chunks[2], app);
    if !matches!(&app.modal, Modal::None) || app.show_help {
        apply_modal_backdrop(f);
    }
    render_modal(f, app);
}

fn render_sidebar(f: &mut Frame, area: Rect, app: &App) {
    let group = group_for_tab(app.tab);
    let views = group_views(group);
    let mut rows: Vec<Line> = views
        .iter()
        .map(|tab| {
            let active = *tab == app.tab;
            Line::from(vec![
                Span::styled(
                    if active { " ▌ " } else { "   " },
                    Style::default().fg(theme::ACCENT),
                ),
                Span::styled(
                    format!("{} {}", TAB_ICONS[*tab], TABS[*tab]),
                    if active {
                        Style::default().fg(theme::ACCENT)
                    } else {
                        theme::subtle()
                    },
                ),
            ])
        })
        .collect();
    rows.extend([
        Line::from(""),
        Line::from(Span::styled(" Tab / ⇧Tab  views", theme::dim())),
        Line::from(Span::styled(" ← / →  sections", theme::dim())),
    ]);
    f.render_widget(
        Paragraph::new(rows)
            .block(theme::panel(GROUPS[group]).border_style(Style::default().fg(theme::SURFACE))),
        area,
    );
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);
    let [indicator, status] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(0)]).areas(left);
    if app.status.ends_with('…') {
        let mut spinner_state = SpinnerState::new(SpinnerType::MiniDot);
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        spinner_state.tick(std::time::Duration::from_millis(
            elapsed.as_millis() as u64 % 1_000,
        ));
        f.render_stateful_widget(
            Spinner::default().style(Style::default().fg(theme::ACCENT)),
            indicator,
            &mut spinner_state,
        );
    } else {
        f.render_widget(
            Paragraph::new(if app.status.starts_with('✗') {
                "✗"
            } else {
                "●"
            })
            .style(Style::default().fg(if app.status.starts_with('✗') {
                theme::RED
            } else {
                theme::GREEN
            })),
            indicator,
        );
    }
    let status_text = app.status.strip_prefix("✗ ").unwrap_or(&app.status);
    f.render_widget(
        Paragraph::new(Span::styled(status_text, theme::subtle())),
        status,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{} / {}  ", GROUPS[group_for_tab(app.tab)], TABS[app.tab]),
                theme::title(),
            ),
            Span::styled(
                if area.width >= 82 {
                    selection_summary(app)
                } else {
                    String::new()
                },
                theme::dim(),
            ),
        ]))
        .alignment(Alignment::Right),
        right,
    );
}

fn selection_summary(app: &App) -> String {
    let len = app.current_len();
    if len == 0 {
        "0 items · no selection".into()
    } else {
        format!("{len} items · row {}", (app.selected + 1).min(len))
    }
}

/// Backdrop treatment inspired by awesome-ratatui's `tui-overlay`: preserve
/// the underlying symbols while pushing their foreground/background down.
fn apply_modal_backdrop(f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = theme::MUTED;
            cell.bg = theme::SURFACE_LO;
        }
    }
}

fn render_window_shadow(f: &mut Frame, card: Rect) {
    let area = f.area();
    let shadow = Rect::new(
        card.x.saturating_add(2),
        card.y.saturating_add(1),
        card.width
            .min(area.right().saturating_sub(card.x.saturating_add(2))),
        card.height
            .min(area.bottom().saturating_sub(card.y.saturating_add(1))),
    );
    f.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme::COLOR0)),
        shadow,
    );
}

fn render_tab(f: &mut Frame, area: Rect, app: &App) {
    match app.tab {
        1 => render_devices(f, area, app),
        2 => render_filesystems(f, area, app),
        3 => render_subvolumes(f, area, app),
        4 => render_snapshots(f, area, app),
        5 => render_shares(f, area, app),
        6 => render_files(f, area, app),
        7 => render_protocols(f, area, app),
        8 => render_users(f, area, app),
        9 => render_alerts(f, area, app),
        10 => render_system(f, area, app),
        _ => render_overview(f, area, app),
    }
}

fn render_modal(f: &mut Frame, app: &App) {
    match &app.modal {
        Modal::Form(form) => render_form(f, f.area(), form),
        Modal::Confirm(confirm) => render_confirm(f, f.area(), confirm),
        Modal::Reveal(reveal) => render_reveal(f, f.area(), reveal),
        Modal::Detail(detail) => render_detail(f, f.area(), detail),
        Modal::Logs(logs) => render_logs(f, f.area(), logs, app),
        Modal::FsStatus(fss) => render_fs_status(f, f.area(), fss, app),
        Modal::CommandPalette(palette) => render_command_palette(f, palette),
        Modal::ContextMenu(menu) => render_context_menu(f, f.area(), menu),
        Modal::None => {
            if app.show_help {
                render_help_popup(f, f.area(), app);
            }
        }
    }
}

fn render_command_palette(f: &mut Frame, palette: &super::app::CommandPalette) {
    let actions = super::app::palette_actions(&palette.query);
    let mut body = format!("{:<42}\n› {}\n\n", "command palette", palette.query);
    if actions.is_empty() {
        body.push_str("  no matching commands");
    } else {
        for (index, action) in actions.iter().take(12).enumerate() {
            let marker = if index == palette.selected {
                "▌"
            } else {
                " "
            };
            body.push_str(&format!("{marker} {}\n", action.label()));
        }
    }
    let popup_body = KnownSizeWrapper {
        inner: Paragraph::new(body),
        width: 46,
        height: (actions.len().min(12) + 4).max(6),
    };
    let popup = Popup::new(popup_body)
        .title(" ↑↓ select · ↵ run · esc close ")
        .style(Style::default().fg(theme::TEXT).bg(theme::SURFACE_LO));
    f.render_widget(&popup, f.area());
}

fn render_context_menu(f: &mut Frame, area: Rect, menu: &super::app::ContextMenu) {
    let [outer] = Layout::vertical([Constraint::Percentage(94)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Percentage(82)])
        .flex(Flex::Center)
        .areas(outer);
    render_window_shadow(f, card);
    f.render_widget(Clear, card);

    let block = theme::panel(&menu.title).border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);
    let [subtitle, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(inner);
    f.render_widget(
        Paragraph::new(Span::styled(&menu.subtitle, theme::subtle())),
        subtitle,
    );

    let [details, actions] =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .spacing(2)
            .areas(body);
    let mut detail_lines = vec![Line::from(Span::styled(
        "status and details",
        theme::title(),
    ))];
    for (key, value) in &menu.details {
        detail_lines.push(Line::from(Span::styled(key, theme::label())));
        detail_lines.push(Line::from(Span::styled(
            format!("  {value}"),
            theme::text(),
        )));
    }
    f.render_widget(
        Paragraph::new(detail_lines)
            .block(theme::panel_bare())
            .wrap(ratatui::widgets::Wrap { trim: false }),
        details,
    );

    let mut action_lines = vec![Line::from(Span::styled("controls", theme::title()))];
    for (index, item) in menu.items.iter().enumerate() {
        let selected = index == menu.selected;
        let marker = if selected { "▌" } else { " " };
        let style = if !item.enabled {
            theme::dim()
        } else if selected {
            Style::default().fg(theme::ACCENT)
        } else {
            theme::text()
        };
        action_lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), Style::default().fg(theme::ACCENT)),
            Span::styled(&item.label, style),
        ]));
        action_lines.push(Line::from(Span::styled(
            format!("    {}", item.hint),
            theme::dim(),
        )));
    }
    f.render_widget(
        Paragraph::new(action_lines)
            .block(theme::panel_bare())
            .wrap(ratatui::widgets::Wrap { trim: false }),
        actions,
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            "↑/↓ choose · enter run · r refresh · esc close",
            theme::dim(),
        ))
        .alignment(Alignment::Center),
        footer,
    );
}

fn render_inspector_content(f: &mut Frame, area: Rect, app: &App) {
    let value: Option<Value> = match app.tab {
        1 => app.devices.get(app.selected).cloned(),
        2 => app.filesystems.get(app.selected).cloned(),
        3 => app.subvolumes.get(app.selected).cloned(),
        4 => app.snapshots.get(app.selected).cloned(),
        5 => app
            .nfs
            .iter()
            .chain(&app.smb)
            .chain(&app.iscsi)
            .chain(&app.nvmeof)
            .nth(app.selected)
            .cloned(),
        6 => app.files.get(app.selected).cloned(),
        7 => app.protocols.get(app.selected).cloned(),
        8 => app
            .users
            .iter()
            .chain(&app.smb_users)
            .chain(&app.smb_groups)
            .chain(&app.tokens)
            .nth(app.selected)
            .cloned(),
        9 => app.alert_rules.get(app.selected).cloned(),
        10 => app
            .system_rows()
            .get(app.selected)
            .map(|row| serde_json::json!({"setting": row.label, "value": row.value})),
        _ => None,
    };
    let actions = match app.tab {
        1 => "↵ controls  t type  w wipe",
        2 => "↵ devices  i status  e edit  m mount  s scrub",
        3 => "e edit  r resize  c clone  s snapshot",
        4 => "c clone  d delete",
        5 => "↵ details  e toggle  n create  d delete",
        6 => "↵ open  ⌫ parent  n mkdir  R rename  d delete",
        7 => "↵ controls  e quick toggle",
        8 => "n create  p password  g/G group member",
        9 => "↵/e toggle  n rule  d delete",
        10 => "↵/e edit or toggle  n add SSH key  d remove key  L logs",
        _ => "",
    };

    let [content, footer] =
        Layout::vertical([Constraint::Min(2), Constraint::Length(4)]).areas(area);
    let text = value.as_ref().map(inspector_text).unwrap_or_else(|| {
        Text::from(vec![Line::from(Span::styled(
            "select an item",
            theme::dim(),
        ))])
    });
    f.render_widget(
        Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: false }),
        content,
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("actions", theme::title())),
            Line::from(Span::styled(actions, theme::dim())),
        ])
        .wrap(ratatui::widgets::Wrap { trim: false }),
        footer,
    );
}

fn inspector_title(app: &App) -> &'static str {
    match app.tab {
        1 => "device inspector",
        2 => "filesystem inspector",
        3 => "subvolume inspector",
        4 => "snapshot inspector",
        5 => "share inspector",
        6 => "file inspector",
        7 => "protocol inspector",
        8 => "identity inspector",
        9 => "alert inspector",
        10 => "system inspector",
        _ => "inspector",
    }
}

fn inspector_text(value: &Value) -> Text<'static> {
    let Some(object) = value.as_object() else {
        return Text::from(value.to_string());
    };
    let mut lines = Vec::new();
    for (key, value) in object {
        let rendered = match value {
            Value::Array(items) => format!("{} items", items.len()),
            Value::Object(items) => format!("{} fields", items.len()),
            Value::Null => "-".into(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        lines.push(Line::from(Span::styled(key.clone(), theme::label())));
        lines.push(Line::from(Span::styled(
            format!("  {rendered}"),
            theme::text(),
        )));
    }
    Text::from(lines)
}

fn render_fs_status(f: &mut Frame, area: Rect, fss: &super::app::FsStatus, app: &App) {
    let [outer] = Layout::vertical([Constraint::Length(23)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(72)])
        .flex(Flex::Center)
        .areas(outer);
    render_window_shadow(f, card);
    f.render_widget(Clear, card);
    let title = format!(
        "{} — s/c scrub · f fsck · R/C toggle jobs · u usage · t top · esc close",
        fss.name
    );
    let block = theme::panel(&title).border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);

    if let Some(raw) = &app.fs_raw {
        f.render_widget(
            Paragraph::new(raw.as_str())
                .style(theme::subtle())
                .wrap(ratatui::widgets::Wrap { trim: false }),
            inner,
        );
        return;
    }

    let [usage_area, gap, scrub_area] = Layout::vertical([
        Constraint::Length(13),
        Constraint::Length(1),
        Constraint::Min(4),
    ])
    .areas(inner);

    // Usage: logical split plus one compact bar per member device.
    let u = app.fs_usage.clone().unwrap_or(Value::Null);
    let devices = u
        .get("devices")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total: u64 = devices
        .iter()
        .filter_map(|d| d.get("total_bytes").and_then(Value::as_u64))
        .sum();
    let used: u64 = devices
        .iter()
        .filter_map(|d| d.get("used_bytes").and_then(Value::as_u64))
        .sum();
    let pct = if total > 0 {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let [u_lines, u_bar, device_lines] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Min(4),
    ])
    .areas(usage_area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            kv("data", &bytes(u.get("data_bytes"))),
            kv("metadata", &bytes(u.get("metadata_bytes"))),
            kv("reserved", &bytes(u.get("reserved_bytes"))),
            kv(
                "physical",
                &format!("{} / {}", bytes_to_human(used), bytes_to_human(total)),
            ),
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
    let device_rows: Vec<Line> = devices
        .iter()
        .take(6)
        .map(|d| {
            let path = field(d, "path");
            let used = d.get("used_bytes").and_then(Value::as_u64).unwrap_or(0);
            let total = d.get("total_bytes").and_then(Value::as_u64).unwrap_or(0);
            let width = 18usize;
            let filled = if total == 0 {
                0
            } else {
                ((used as f64 / total as f64) * width as f64).round() as usize
            }
            .min(width);
            Line::from(vec![
                Span::styled(format!(" {path:<18}"), theme::label()),
                Span::styled("█".repeat(filled), Style::default().fg(theme::ACCENT)),
                Span::styled("░".repeat(width - filled), theme::dim()),
                Span::styled(format!(" {}", bytes_to_human(used)), theme::subtle()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(device_rows), device_lines);
    let _ = gap;

    // Background-operation status strip.
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
            Line::from(vec![
                Span::styled(" reconcile ", theme::title()),
                Span::styled(
                    if app
                        .fs_reconcile
                        .as_ref()
                        .and_then(|v| v.get("enabled"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        "on"
                    } else {
                        "off"
                    },
                    theme::subtle(),
                ),
                Span::styled("  copygc ", theme::title()),
                Span::styled(
                    match app.fs_copygc.as_ref().and_then(Value::as_bool) {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "n/a",
                    },
                    theme::subtle(),
                ),
                Span::styled("  fsck ", theme::title()),
                Span::styled(
                    if app
                        .fs_fsck
                        .as_ref()
                        .and_then(|v| v.get("running"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        "running"
                    } else {
                        "idle"
                    },
                    theme::subtle(),
                ),
            ]),
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
    render_window_shadow(f, card);
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
    render_window_shadow(f, card);
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
                let unhealthy = r.iter().any(|c| c.contains("MISSING"))
                    || r.iter().any(|c| c.starts_with('r') && c != "r0 w0 c0");
                let row = Row::new(
                    r.iter()
                        .map(|c| Cell::from(format!(" {c}")))
                        .collect::<Vec<_>>(),
                );
                if unhealthy {
                    row.style(Style::default().fg(theme::RED))
                } else {
                    row
                }
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
    render_window_shadow(f, card);
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
    let labels = if area.width >= 82 {
        Some(&GROUPS)
    } else if area.width >= 45 {
        Some(&GROUPS_COMPACT)
    } else {
        None
    };
    if let Some(labels) = labels {
        let group = group_for_tab(app.tab);
        f.render_widget(
            TabNav::new(labels, group)
                .style(theme::subtle())
                .highlight_style(Style::default().fg(theme::ACCENT))
                .highlight_bold(false)
                .border_style(Style::default().fg(theme::SURFACE))
                .indicator(Some("●")),
            area,
        );
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " nastty ",
                Style::default().fg(theme::COLOR0).bg(theme::ACCENT),
            ),
            Span::styled(
                format!(" {} / {} ", GROUPS[group_for_tab(app.tab)], TABS[app.tab]),
                theme::title(),
            ),
        ]))
        .block(theme::panel_bare()),
        area,
    );
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
                "metrics are starting — nasttyd collects them automatically",
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
    let compact = inner.width < 48;

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
        &if compact {
            format!("cpu  {load1:.1}/{cores:.0}{temp}")
        } else {
            format!("cpu  load {load1:.1}/{cores:.0}{temp}")
        },
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
            "mem  {}{}{}",
            human_bytes(mem_used),
            if compact { "/" } else { " / " },
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
                "{}  {}{}{}",
                field(fs, "name"),
                human_bytes(used),
                if compact { "/" } else { " / " },
                human_bytes(total)
            ),
            pct,
            color,
        );
    }
}

fn gauge_line(f: &mut Frame, area: Rect, label: &str, pct: f64, color: ratatui::style::Color) {
    let label_width = if area.width >= 48 { 30 } else { 22 };
    let [name, bar] = Layout::horizontal([Constraint::Length(label_width), Constraint::Min(8)])
        .spacing(1)
        .areas(area);
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
        Line::from(vec![kv_key("bcachefs"), bcachefs_span(&info)]),
        kv("kvm", &field(&info, "kvm_available")),
    ];
    if !bcachefs_available(&info) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ✗ bcachefs unavailable — install bcachefs-tools + kernel module",
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

fn bcachefs_span(info: &Value) -> Span<'static> {
    match bcachefs_version(info) {
        Some(v) => Span::styled(format!("● {v}"), Style::default().fg(theme::GREEN)),
        None => Span::styled("✗ not available", Style::default().fg(theme::RED)),
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
            "devices ({}) — ↵ controls · w wipe · t type",
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
    let empty_text = match &app.system_info {
        Some(info) if !bcachefs_available(info) => {
            "bcachefs not available — install bcachefs-tools and the kernel module (see README)"
        }
        _ => "no filesystems — create one with fs.create",
    };
    let rows: Vec<Row> = app
        .filesystems
        .iter()
        .map(|fs| {
            let mounted = fs.get("mounted").and_then(|v| v.as_bool()).unwrap_or(false);
            let name = field(fs, "name");
            let jobs = app.fs_strip.get(&name);
            let on_off = |slot: usize, nested: bool| {
                jobs.and_then(|s| s[slot].as_ref())
                    .and_then(|v| if nested { v.get("enabled") } else { Some(v) })
                    .and_then(Value::as_bool)
                    .map(|v| if v { "on" } else { "off" })
                    .unwrap_or("-")
            };
            let running = |slot: usize, label: &str| {
                jobs.and_then(|s| s[slot].as_ref())
                    .filter(|v| v.get("running").and_then(Value::as_bool).unwrap_or(false))
                    .map(|v| {
                        let pct = v
                            .get("progress_percent")
                            .and_then(Value::as_f64)
                            .map(|p| format!(" {p:.0}%"))
                            .unwrap_or_default();
                        format!("{label}{pct}")
                    })
            };
            let active = running(2, "scrub")
                .or_else(|| running(3, "fsck"))
                .unwrap_or_else(|| "idle".into());
            Row::new(vec![
                cell2(primary(name), secondary(field(fs, "mount_point"))),
                cell1(theme::badge(mounted, "mounted", "unmounted")),
                cell1(Span::styled(field(fs, "state"), theme::subtle())),
                cell1(Span::styled(
                    if mounted {
                        format!(
                            "rec {} · gc {} · {active}",
                            on_off(0, true),
                            on_off(1, false)
                        )
                    } else {
                        "-".into()
                    },
                    theme::subtle(),
                )),
            ])
            .height(2)
        })
        .collect();
    render_table(
        f,
        area,
        &format!(
            "filesystems ({}) — ↵ members · i status · e edit · m mount · s scrub",
            app.filesystems.len()
        ),
        &["filesystem", "status", "state", "background work"],
        &[
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(18),
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
        "protocols — ↵ service controls · e quick toggle",
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
        &["snapshot", "subvolume"],
        &[Constraint::Min(30), Constraint::Length(24)],
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
    // The filesystem-create form keeps its advanced fields collapsed until
    // F2 moves focus into that page. Long forms use a focus-following window.
    let (start, end) = if form.fields.len() == 21 && form.focus < 6 {
        (0, 6)
    } else {
        let max_visible = area.height.saturating_sub(8).max(1) as usize;
        let start = form
            .focus
            .saturating_sub(max_visible / 2)
            .min(form.fields.len().saturating_sub(max_visible));
        (start, (start + max_visible).min(form.fields.len()))
    };
    let visible = &form.fields[start..end];
    let height = (visible.len() as u16) + 6;
    let [outer] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [card] = Layout::horizontal([Constraint::Length(58)])
        .flex(Flex::Center)
        .areas(outer);
    render_window_shadow(f, card);
    f.render_widget(Clear, card);

    let block = theme::panel(&form.title)
        .title(
            Line::from(Span::styled(
                format!(" field {} / {} ", form.focus + 1, form.fields.len()),
                theme::dim(),
            ))
            .right_aligned(),
        )
        .border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let mut constraints = vec![Constraint::Length(1)]; // top spacer
    constraints.extend(vec![Constraint::Length(1); visible.len()]);
    constraints.push(Constraint::Length(1)); // spacer
    constraints.push(Constraint::Length(1)); // hint
    constraints.push(Constraint::Min(0)); // keys
    let rows = Layout::vertical(constraints).split(inner);

    for (i, fld) in visible.iter().enumerate() {
        let focused = start + i == form.focus;
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
        rows[visible.len() + 2],
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
        rows[visible.len() + 3],
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
    render_window_shadow(f, card);
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
    let row_count = rows.len();
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
        if row_count > 10 {
            let mut scrollbar = ScrollbarState::new(row_count).position(selected);
            f.render_stateful_widget(
                Scrollbar::default()
                    .orientation(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some("│"))
                    .thumb_symbol("█")
                    .track_style(theme::dim())
                    .thumb_style(Style::default().fg(theme::ACCENT)),
                area.inner(Margin {
                    vertical: 1,
                    horizontal: 0,
                }),
                &mut scrollbar,
            );
            let mut paginator_state = PaginatorState::new(row_count, 10);
            for _ in 0..(selected / 10) {
                paginator_state.next_page();
            }
            let pager_width = 12.min(area.width);
            let pager_area = Rect::new(
                area.x + area.width.saturating_sub(pager_width) / 2,
                area.bottom().saturating_sub(1),
                pager_width,
                1,
            );
            f.render_stateful_widget(
                Paginator::default().mode(PaginatorMode::Arabic),
                pager_area,
                &mut paginator_state,
            );
        }
    }
}

fn render_help_popup(f: &mut Frame, area: Rect, app: &App) {
    let contextual: &[(&str, &str)] = match app.tab {
        0 => &[("r", "refresh dashboard data")],
        1 => &[
            ("enter", "open device status and control menu"),
            ("w", "wipe selected unused device"),
            ("t", "change selected device class"),
        ],
        2 => &[
            ("enter", "open member-device window"),
            ("n", "create filesystem"),
            ("e", "edit filesystem options"),
            ("m", "mount or unmount filesystem"),
            ("i", "open usage and job-status window"),
            ("s", "start scrub"),
            ("D", "destroy filesystem with confirmation"),
        ],
        3 => &[
            ("n", "create subvolume"),
            ("e", "edit subvolume properties"),
            ("r", "resize subvolume"),
            ("c", "clone subvolume"),
            ("s", "snapshot selected subvolume"),
            ("d", "delete selected subvolume"),
        ],
        4 => &[
            ("n", "create snapshot"),
            ("c", "clone selected snapshot"),
            ("d", "delete selected snapshot"),
        ],
        5 => &[
            ("enter", "open share details and members"),
            ("n", "create share or block export"),
            ("e", "enable or disable selected share"),
            ("d", "delete selected share"),
        ],
        6 => &[
            ("enter", "open directory or selected file"),
            ("backspace", "go to parent directory"),
            ("n", "create directory"),
            ("R", "rename selected entry"),
            ("d", "delete selected entry"),
        ],
        7 => &[
            ("enter", "open installation, status, and control menu"),
            ("e", "quick enable or disable selected service"),
        ],
        8 => &[
            ("n", "create user, SMB identity, group, or token"),
            ("p", "change selected user password"),
            ("g", "add user to selected group"),
            ("G", "remove user from selected group"),
            ("d", "delete or revoke selected identity"),
        ],
        9 => &[
            ("enter / e", "enable or disable selected alert rule"),
            ("n", "create alert rule"),
            ("d", "delete selected alert rule"),
        ],
        10 => &[
            ("enter / e", "edit or toggle selected setting"),
            ("n", "add an SSH authorized key"),
            ("L", "open system log window"),
        ],
        _ => &[],
    };
    let height = (contextual.len() + 20).min(area.height.saturating_sub(2) as usize) as u16;
    let [outer] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::horizontal([Constraint::Length(64)])
        .flex(Flex::Center)
        .areas(outer);

    render_window_shadow(f, popup);
    f.render_widget(Clear, popup);

    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} workspace", TABS[app.tab]),
            theme::title(),
        )),
        Line::from(""),
    ];
    lines.extend(
        contextual
            .iter()
            .map(|(key, description)| shortcut(key, description)),
    );
    lines.extend([
        Line::from(""),
        shortcut("?", "show / close this help"),
        shortcut("/ or :", "open command palette"),
        shortcut("mouse", "click tabs/views/rows, wheel selection"),
        shortcut("1-9 / 0", "jump directly to a view"),
        shortcut("tab / shift-tab", "next / previous view in section"),
        shortcut("← →  h l", "previous / next section"),
        shortcut("[ / ]", "view aliases for tab / shift-tab"),
        shortcut("space", "open / close inspector drawer"),
        shortcut("↑ ↓  k j", "move selection"),
        shortcut("r", "refresh all data"),
        shortcut("q", "quit"),
        shortcut("ctrl-c", "quit immediately"),
        shortcut("esc", "close help, or quit when help is closed"),
    ]);

    let title = format!("{} shortcuts", TABS[app.tab]);
    f.render_widget(
        Paragraph::new(lines)
            .block(theme::panel(&title))
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
    fn selected_tab_uses_accent_text_without_a_background_fill() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let app = App::for_test();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let selected: Vec<_> = buf
            .content()
            .iter()
            .take(100 * 3)
            .filter(|cell| cell.fg == theme::ACCENT && !cell.symbol().trim().is_empty())
            .collect();

        assert!(!selected.is_empty(), "selected tab cells not found");
        assert!(
            selected.iter().all(|cell| cell.bg != theme::ACCENT),
            "selected tabs must not have an accent background fill"
        );
        assert!(
            selected
                .iter()
                .all(|cell| !cell.modifier.contains(Modifier::BOLD)),
            "selected-tab text must stay plain accent text"
        );
    }

    #[test]
    fn enter_on_device_opens_a_real_control_menu() {
        let mut app = App::for_test();
        app.tab = 1;
        app.devices = vec![serde_json::json!({
            "path":"/dev/sda", "model":"Example SSD", "serial":"ABC123",
            "device_class":"ssd", "dev_type":"disk", "transport":"sata",
            "size_bytes":1_000_000_000_000u64, "in_use":false
        })];
        super::super::app::open_device_menu(&mut app);
        let text = buffer_text(&app, 110, 28);
        assert!(text.contains("Device controls · /dev/sda"));
        assert!(text.contains("Change storage class"));
        assert!(text.contains("Wipe device"));
        assert!(text.contains("Refresh device and SMART status"));
    }

    #[test]
    fn enter_on_protocol_opens_installation_status_and_controls() {
        let mut app = App::for_test();
        app.tab = 7;
        app.protocols = vec![serde_json::json!({
            "name":"nfs", "display_name":"NFS", "enabled":false, "running":false,
            "installed":true, "package":"nfs-kernel-server", "binary":"exportfs",
            "units":["nfs-server.service"],
            "configuration":"/etc/exports.d/nasty.exports",
            "controls":"shares, clients, read-only mode, RDMA, nfsd tuning",
            "description":"Unix/Linux network filesystem sharing"
        })];
        super::super::app::open_protocol_menu(&mut app);
        let text = buffer_text(&app, 110, 30);
        assert!(text.contains("Service controls · NFS"));
        assert!(text.contains("installed (nfs-kernel-server)"));
        assert!(text.contains("nfs-server.service"));
        assert!(text.contains("Enable service"));
        assert!(text.contains("Open related controls"));
    }

    #[test]
    fn wide_boxed_tabs_and_error_status_render() {
        let mut app = App::for_test();
        app.tab = 2;
        app.status = "✗ filesystem request failed".into();
        let text = buffer_text(&app, 180, 28);
        assert!(text.contains("Filesystems"), "boxed active tab missing");
        assert!(
            text.contains("filesystem request failed"),
            "error status missing"
        );
        assert!(text.contains("╭"), "boxed tab/window chrome missing");
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
    fn wide_workspace_renders_inspector_pane() {
        let mut app = App::for_test();
        app.tab = 2;
        app.inspector_open = true;
        app.filesystems = vec![serde_json::json!({
            "name":"tank", "mounted":true, "mount_point":"/fs/tank"
        })];
        let text = buffer_text(&app, 140, 24);
        assert!(text.contains("filesystem inspector"));
        assert!(text.contains("actions"));
        assert!(text.contains("/fs/tank"));
    }

    #[test]
    fn inspector_drawer_is_hidden_until_requested() {
        let mut app = App::for_test();
        app.tab = 2;
        app.filesystems = vec![serde_json::json!({
            "name":"tank", "mounted":true, "mount_point":"/fs/tank"
        })];
        let text = buffer_text(&app, 140, 24);
        assert!(!text.contains("filesystem inspector"));

        app.inspector_open = true;
        let text = buffer_text(&app, 140, 24);
        assert!(text.contains("filesystem inspector"));
    }

    #[test]
    fn renders_awesome_ratatui_command_popup() {
        let mut app = App::for_test();
        app.modal = Modal::CommandPalette(super::super::app::CommandPalette {
            query: "device".into(),
            selected: 0,
        });
        let text = buffer_text(&app, 100, 24);
        assert!(text.contains("command palette"));
        assert!(text.contains("go to Devices"), "{text}");
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

        app.tab = 2;
        let text = buffer_text(&app, 100, 26);
        assert!(text.contains("Filesystems shortcuts"));
        assert!(text.contains("open member-device window"));
        assert!(text.contains("destroy filesystem with confirmation"));
    }

    #[test]
    fn every_view_has_its_own_help_window() {
        let mut app = App::for_test();
        app.show_help = true;
        for (tab, name) in TABS.iter().enumerate() {
            app.tab = tab;
            let text = buffer_text(&app, 100, 32);
            assert!(
                text.contains(&format!("{name} shortcuts")),
                "missing contextual help window for {name}"
            );
        }
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
            "kvm_available": true,
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
            serde_json::json!({"name":"nfs","display_name":"NFS","enabled":true,"running":true,
                "installed":true,"package":"nfs-kernel-server","binary":"exportfs",
                "units":["nfs-server.service"],"configuration":"/etc/exports.d/nasty.exports",
                "controls":"shares, clients, read-only mode, RDMA, nfsd tuning",
                "description":"Unix/Linux network filesystem sharing"}),
            serde_json::json!({"name":"smb","display_name":"SMB","enabled":false,"running":false,
                "installed":false,"package":"samba","binary":"smbd",
                "units":["samba-smbd.service"],"configuration":"/etc/samba/smb.nasty.conf",
                "controls":"shares, users, guest policy, Time Machine, tuning",
                "description":"Windows and macOS file sharing"}),
            serde_json::json!({"name":"ssh","display_name":"SSH","enabled":true,"running":true,
                "installed":true,"package":"openssh-server","binary":"sshd",
                "units":["sshd.service"],"configuration":"/etc/ssh/sshd_config",
                "controls":"password authentication and authorized keys",
                "description":"Secure shell access"}),
        ];
        app.users = vec![
            serde_json::json!({"username":"admin","role":"admin","must_change_password":false}),
            serde_json::json!({"username":"kush","role":"operator","must_change_password":true}),
        ];
        app.smb_users = vec![serde_json::json!({"username":"media"})];
        app.smb_groups = vec![serde_json::json!({"name":"family"})];
        app.snapshots = vec![serde_json::json!({
            "name":"data@daily","filesystem":"tank","subvolume":"data"
        })];
        app.stats = Some(serde_json::json!({
            "cpu": {"count": 24, "load_1": 3.6, "load_5": 2.0, "load_15": 1.2, "temp_c": 44},
            "memory": {"total_bytes": 67108864000u64, "used_bytes": 21474836480u64},
        }));
        app.cpu_history = vec![5, 8, 12, 20, 15, 30, 25, 40, 35, 20, 18, 22, 30, 15];
        app.mem_history = vec![30, 31, 32, 32, 33, 31, 30, 32, 34, 33, 32, 31, 32, 32];
        app.filesystems = vec![serde_json::json!({
            "name":"tank","mounted":true,"mount_point":"/fs/tank",
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
        let preview_width = std::env::var("NASTTY_PREVIEW_WIDTH")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(110);

        for (tab, tab_name) in TABS.iter().enumerate() {
            app.tab = tab;
            app.selected = 1.min(app.current_len().saturating_sub(1));
            let mut terminal = Terminal::new(TestBackend::new(preview_width, 26)).unwrap();
            terminal.draw(|f| render_app(f, &app)).unwrap();
            let buf = terminal.backend().buffer().clone();
            println!("── tab {tab} ({tab_name}) ──");
            for y in 0..buf.area.height {
                let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                println!("{line}");
            }
            println!();
        }

        app.tab = 2;
        app.selected = 0;
        app.inspector_open = true;
        let mut terminal = Terminal::new(TestBackend::new(preview_width, 26)).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("── overlay (Filesystem inspector) ──");
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{line}");
        }

        app.inspector_open = false;
        app.show_help = true;
        let mut terminal = Terminal::new(TestBackend::new(preview_width, 30)).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("── popup (Filesystem help) ──");
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{line}");
        }

        app.show_help = false;
        app.tab = 1;
        app.selected = 0;
        super::super::app::open_device_menu(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(preview_width, 30)).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("── menu (Device controls) ──");
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{line}");
        }

        app.modal = Modal::None;
        app.tab = 7;
        app.selected = 0;
        super::super::app::open_protocol_menu(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(preview_width, 30)).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        println!("── menu (Service controls) ──");
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{line}");
        }
    }
}
