//! Login and forced password-change screens.

use futures_util::SinkExt;
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};
use tokio::sync::mpsc;
use tui_big_text::{BigText, PixelSize};

use crate::client::{self, WsAck, WsStream};

use super::{Term, theme};

/// Which field the login form has focused.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    User,
    Pass,
}

struct LoginForm {
    user: String,
    pass: String,
    focus: Field,
    status: String,
    error: bool,
    busy: bool,
}

impl LoginForm {
    fn new(user: Option<String>) -> Self {
        let user = user.unwrap_or_else(|| "admin".to_string());
        Self {
            focus: if user.is_empty() {
                Field::User
            } else {
                Field::Pass
            },
            user,
            pass: String::new(),
            status: String::new(),
            error: false,
            busy: false,
        }
    }
}

/// Run the login screen until we have an authenticated WebSocket, or the
/// user quits (returns `None`).
pub async fn login_flow(
    terminal: &mut Term,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
    base: &str,
    user: Option<String>,
) -> Result<Option<(WsStream, WsAck)>, Box<dyn std::error::Error>> {
    let mut form = LoginForm::new(user);

    loop {
        terminal.draw(|f| render_login(f, &form, base))?;

        let Some(ev) = input_rx.recv().await else {
            return Ok(None); // input thread gone
        };
        let Event::Key(key) = ev else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                form.focus = match form.focus {
                    Field::User => Field::Pass,
                    Field::Pass => Field::User,
                };
            }
            KeyCode::Backspace => {
                match form.focus {
                    Field::User => form.user.pop(),
                    Field::Pass => form.pass.pop(),
                };
            }
            KeyCode::Char(c) => match form.focus {
                Field::User => form.user.push(c),
                Field::Pass => form.pass.push(c),
            },
            KeyCode::Enter => {
                form.busy = true;
                form.error = false;
                form.status = "connecting…".to_string();
                terminal.draw(|f| render_login(f, &form, base))?;

                match try_connect(base, &form.user, &form.pass).await {
                    Err(e) => {
                        form.status = e;
                        form.error = true;
                        form.busy = false;
                    }
                    Ok((ws, ack)) if ack.must_change_password => {
                        match change_password_flow(terminal, input_rx, ws, &form.pass).await? {
                            Some(pair) => return Ok(Some(pair)),
                            None => {
                                // user cancelled the change; back to login
                                form.status = "password change cancelled".to_string();
                                form.error = true;
                                form.busy = false;
                            }
                        }
                    }
                    Ok(pair) => return Ok(Some(pair)),
                }
            }
            _ => {}
        }
    }
}

async fn try_connect(base: &str, user: &str, pass: &str) -> Result<(WsStream, WsAck), String> {
    let token = client::login(base, user, pass).await?;
    client::connect_ws(base, &token).await
}

/// Forced password-change screen, shown when the server reports
/// `must_change_password`. Owns the WebSocket exclusively while it sends
/// `auth.change_password` and reads the reply.
async fn change_password_flow(
    terminal: &mut Term,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
    mut ws: WsStream,
    old_password: &str,
) -> Result<Option<(WsStream, WsAck)>, Box<dyn std::error::Error>> {
    let mut new = String::new();
    let mut confirm = String::new();
    let mut focus_new = true;
    let mut status = "set a new password (min 8 characters)".to_string();
    let mut error = false;

    loop {
        terminal.draw(|f| render_change(f, &new, &confirm, focus_new, &status, error))?;

        let Some(ev) = input_rx.recv().await else {
            return Ok(None);
        };
        let Event::Key(key) = ev else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Tab | KeyCode::Up | KeyCode::Down => focus_new = !focus_new,
            KeyCode::Backspace => {
                if focus_new {
                    new.pop();
                } else {
                    confirm.pop();
                }
            }
            KeyCode::Char(c) => {
                if focus_new {
                    new.push(c);
                } else {
                    confirm.push(c);
                }
            }
            KeyCode::Enter => {
                if new != confirm {
                    status = "passwords do not match".to_string();
                    error = true;
                    continue;
                }
                if new.len() < 8 {
                    status = "password must be at least 8 characters".to_string();
                    error = true;
                    continue;
                }
                status = "changing…".to_string();
                error = false;
                terminal.draw(|f| render_change(f, &new, &confirm, focus_new, &status, error))?;

                let req = client::request(
                    0,
                    "auth.change_password",
                    serde_json::json!({ "old_password": old_password, "new_password": new }),
                );
                if let Err(e) = ws.send(req).await {
                    status = format!("send failed: {e}");
                    error = true;
                    continue;
                }
                match client::next_text(&mut ws).await {
                    None => return Ok(None),
                    Some(text) => match client::parse_incoming(&text) {
                        client::Incoming::Response { result: Ok(_), .. } => {
                            // Password changed; the existing session is now
                            // unblocked. Report the fresh ack.
                            let ack = WsAck {
                                authenticated: true,
                                username: String::new(),
                                role: String::new(),
                                must_change_password: false,
                            };
                            return Ok(Some((ws, ack)));
                        }
                        client::Incoming::Response { result: Err(e), .. } => {
                            status = e;
                            error = true;
                        }
                        _ => {
                            status = "unexpected server reply".to_string();
                            error = true;
                        }
                    },
                }
            }
            _ => {}
        }
    }
}

// ── rendering ───────────────────────────────────────────────────

fn render_login(f: &mut Frame, form: &LoginForm, base: &str) {
    // Vertical stack, centered: logo, card, server line.
    let [logo_area, card_area, server_area] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(1),
    ])
    .flex(Flex::Center)
    .areas(f.area());

    render_logo(f, logo_area);

    // The card itself, centered horizontally.
    let [card] = Layout::horizontal([Constraint::Length(56)])
        .flex(Flex::Center)
        .areas(card_area);
    f.render_widget(Clear, card);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(Span::styled(" sign in ", theme::title()))
        .title_alignment(Alignment::Center)
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(card);
    f.render_widget(block, card);

    let rows = Layout::vertical([
        Constraint::Length(1), // user
        Constraint::Length(1), // pass
        Constraint::Length(1), // spacer
        Constraint::Length(1), // status
        Constraint::Min(0),    // help
    ])
    .split(inner);

    f.render_widget(
        field_line("user", &form.user, false, form.focus == Field::User),
        rows[0],
    );
    f.render_widget(
        field_line("pass", &form.pass, true, form.focus == Field::Pass),
        rows[1],
    );
    f.render_widget(status_line(&form.status, form.error, form.busy), rows[3]);
    f.render_widget(
        Paragraph::new(Line::from(
            [
                theme::chip("tab", "switch"),
                theme::chip("enter", "sign in"),
                theme::chip("esc", "quit"),
            ]
            .concat(),
        ))
        .alignment(Alignment::Center),
        rows[4],
    );

    f.render_widget(
        Paragraph::new(Span::styled(base.to_string(), theme::dim())).alignment(Alignment::Center),
        server_area,
    );
}

fn render_change(
    f: &mut Frame,
    new: &str,
    confirm: &str,
    focus_new: bool,
    status: &str,
    error: bool,
) {
    let [logo_area, card_area] = Layout::vertical([Constraint::Length(9), Constraint::Length(10)])
        .flex(Flex::Center)
        .areas(f.area());

    render_logo(f, logo_area);

    let [card] = Layout::horizontal([Constraint::Length(60)])
        .flex(Flex::Center)
        .areas(card_area);
    f.render_widget(Clear, card);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::YELLOW))
        .title(Span::styled(" change password — required ", theme::title()))
        .title_alignment(Alignment::Center)
        .padding(Padding::new(2, 2, 1, 0));
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

    f.render_widget(field_line("new    ", new, true, focus_new), rows[0]);
    f.render_widget(field_line("confirm", confirm, true, !focus_new), rows[1]);
    f.render_widget(status_line(status, error, false), rows[3]);
    f.render_widget(
        Paragraph::new(Line::from(
            [
                theme::chip("tab", "switch"),
                theme::chip("enter", "submit"),
                theme::chip("esc", "cancel"),
            ]
            .concat(),
        ))
        .alignment(Alignment::Center),
        rows[4],
    );
}

/// Big two-tone "NASTTY" logo.
fn render_logo(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled("NAS", Style::default().fg(theme::MAUVE)),
        Span::styled("TTY", Style::default().fg(theme::ACCENT)),
    ]);
    let big = BigText::builder()
        .pixel_size(PixelSize::Full)
        .lines(vec![line])
        .alignment(Alignment::Center)
        .build();
    f.render_widget(big, area);
    // Tagline under the logo, in the last row of the logo area.
    if area.height >= 9 {
        let tag = Rect {
            y: area.y + 8,
            height: 1,
            ..area
        };
        f.render_widget(
            Paragraph::new(Span::styled("· your disks, your rules ·", theme::dim()))
                .alignment(Alignment::Center),
            tag,
        );
    }
}

fn field_line<'a>(label: &'a str, value: &str, secret: bool, focused: bool) -> Paragraph<'a> {
    let shown = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let marker = if focused { "▌ " } else { "  " };
    let value_style = if focused {
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::subtle()
    };
    let cursor = if focused {
        Span::styled("█", Style::default().fg(theme::ACCENT))
    } else {
        Span::raw("")
    };
    Paragraph::new(Line::from(vec![
        Span::styled(marker, Style::default().fg(theme::ACCENT)),
        Span::styled(format!("{label}  "), theme::label()),
        Span::styled(shown, value_style),
        cursor,
    ]))
}

fn status_line<'a>(status: &str, error: bool, busy: bool) -> Paragraph<'a> {
    let style = if error {
        Style::default().fg(theme::RED)
    } else if busy {
        Style::default().fg(theme::YELLOW)
    } else {
        theme::dim()
    };
    let prefix = if error {
        "✗ "
    } else if busy {
        "◌ "
    } else {
        ""
    };
    Paragraph::new(Span::styled(format!("{prefix}{status}"), style)).alignment(Alignment::Center)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Visual preview of the login screen. Run with:
    /// `cargo test --lib -- --ignored login_preview --nocapture`
    #[test]
    #[ignore]
    fn login_preview() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut form = LoginForm::new(Some("admin".to_string()));
        form.pass = "secret".to_string();
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal
            .draw(|f| render_login(f, &form, "http://127.0.0.1:2137"))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            println!("{line}");
        }
    }
}
