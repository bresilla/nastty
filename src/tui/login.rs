//! Login and forced password-change screens.

use futures_util::SinkExt;
use ratatui::Frame;
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tokio::sync::mpsc;

use crate::client::{self, WsAck, WsStream};

use super::Term;

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
        terminal.draw(|f| render_login(f, &form))?;

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
                form.status = "connecting…".to_string();
                terminal.draw(|f| render_login(f, &form))?;

                match try_connect(base, &form.user, &form.pass).await {
                    Err(e) => {
                        form.status = e;
                        form.busy = false;
                    }
                    Ok((ws, ack)) if ack.must_change_password => {
                        match change_password_flow(terminal, input_rx, ws, &form.pass).await? {
                            Some(pair) => return Ok(Some(pair)),
                            None => {
                                // user cancelled the change; back to login
                                form.status = "password change cancelled".to_string();
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

    loop {
        terminal.draw(|f| render_change(f, &new, &confirm, focus_new, &status))?;

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
                    continue;
                }
                if new.len() < 8 {
                    status = "password must be at least 8 characters".to_string();
                    continue;
                }
                status = "changing…".to_string();
                terminal.draw(|f| render_change(f, &new, &confirm, focus_new, &status))?;

                let req = client::request(
                    0,
                    "auth.change_password",
                    serde_json::json!({ "old_password": old_password, "new_password": new }),
                );
                if let Err(e) = ws.send(req).await {
                    status = format!("send failed: {e}");
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
                        }
                        _ => status = "unexpected server reply".to_string(),
                    },
                }
            }
            _ => {}
        }
    }
}

fn render_login(f: &mut Frame, form: &LoginForm) {
    let area = centered(f.area(), 52, 11);
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" nastty — sign in ")
        .title_alignment(Alignment::Center);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // spacer
        Constraint::Length(1), // user
        Constraint::Length(1), // pass
        Constraint::Length(1), // spacer
        Constraint::Length(1), // status
        Constraint::Min(0),    // help
    ])
    .split(inner);

    f.render_widget(
        field_line("User", &form.user, false, form.focus == Field::User),
        rows[1],
    );
    f.render_widget(
        field_line("Pass", &form.pass, true, form.focus == Field::Pass),
        rows[2],
    );
    f.render_widget(
        Paragraph::new(form.status.clone()).style(Style::default().fg(Color::Yellow)),
        rows[4],
    );
    f.render_widget(
        Paragraph::new("Tab switch · Enter submit · Esc quit")
            .style(Style::default().fg(Color::DarkGray)),
        rows[5],
    );
}

fn render_change(f: &mut Frame, new: &str, confirm: &str, focus_new: bool, status: &str) {
    let area = centered(f.area(), 56, 11);
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" change password (required) ")
        .title_alignment(Alignment::Center);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    f.render_widget(field_line("New    ", new, true, focus_new), rows[1]);
    f.render_widget(field_line("Confirm", confirm, true, !focus_new), rows[2]);
    f.render_widget(
        Paragraph::new(status.to_string()).style(Style::default().fg(Color::Yellow)),
        rows[4],
    );
    f.render_widget(
        Paragraph::new("Tab switch · Enter submit · Esc cancel")
            .style(Style::default().fg(Color::DarkGray)),
        rows[5],
    );
}

fn field_line<'a>(label: &'a str, value: &str, secret: bool, focused: bool) -> Paragraph<'a> {
    let shown = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let marker = if focused { "▶ " } else { "  " };
    let value_style = if focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{marker}{label}: "),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(shown, value_style),
    ]))
}

/// Center a `w`×`h` rectangle inside `area`.
fn centered(area: ratatui::layout::Rect, w: u16, h: u16) -> ratatui::layout::Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    ratatui::layout::Rect {
        x,
        y,
        width: w,
        height: h,
    }
}
