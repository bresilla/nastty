//! Main application: tabbed live views over the NAS state, driven by
//! JSON-RPC responses and server events.

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::client::{self, Incoming, WsAck, WsStream};

use super::Term;

pub(super) type WsWrite = SplitSink<WsStream, Message>;

pub(super) const TABS: [&str; 6] = [
    "Overview",
    "Devices",
    "Filesystems",
    "Subvolumes",
    "Shares",
    "Protocols",
];
const TAB_DEVICES: usize = 1;
const TAB_FILESYSTEMS: usize = 2;
const TAB_SUBVOLUMES: usize = 3;
const TAB_SHARES: usize = 4;
const TAB_PROTOCOLS: usize = 5;

// Stable request ids: one per query kind, so responses route without a
// pending-request map.
const ID_ME: i64 = 101;
const ID_SYSINFO: i64 = 100;
const ID_DEVICES: i64 = 1;
const ID_FS: i64 = 2;
const ID_SUBVOL: i64 = 3;
const ID_NFS: i64 = 4;
const ID_SMB: i64 = 5;
const ID_PROTO: i64 = 6;
const ID_ACTION: i64 = 200;

pub(super) struct App {
    pub username: String,
    pub role: String,
    pub connected: bool,
    pub tab: usize,
    pub selected: usize,
    pub system_info: Option<Value>,
    pub devices: Vec<Value>,
    pub filesystems: Vec<Value>,
    pub subvolumes: Vec<Value>,
    pub nfs: Vec<Value>,
    pub smb: Vec<Value>,
    pub protocols: Vec<Value>,
    pub status: String,
    pub show_help: bool,
    should_quit: bool,
}

impl App {
    pub(super) fn new(ack: WsAck) -> Self {
        Self {
            username: ack.username,
            role: ack.role,
            connected: true,
            tab: 0,
            selected: 0,
            system_info: None,
            devices: Vec::new(),
            filesystems: Vec::new(),
            subvolumes: Vec::new(),
            nfs: Vec::new(),
            smb: Vec::new(),
            protocols: Vec::new(),
            status: "loading…".to_string(),
            show_help: false,
            should_quit: false,
        }
    }

    #[cfg(test)]
    pub(super) fn for_test() -> Self {
        Self::new(WsAck {
            authenticated: true,
            username: String::new(),
            role: String::new(),
            must_change_password: false,
        })
    }

    /// Length of the list shown on the current tab (for selection clamping).
    pub(super) fn current_len(&self) -> usize {
        match self.tab {
            TAB_DEVICES => self.devices.len(),
            TAB_FILESYSTEMS => self.filesystems.len(),
            TAB_SUBVOLUMES => self.subvolumes.len(),
            TAB_SHARES => self.nfs.len() + self.smb.len(),
            TAB_PROTOCOLS => self.protocols.len(),
            _ => 0,
        }
    }
}

/// Run the main view loop until the user quits.
pub(super) async fn run_app(
    terminal: &mut Term,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
    ws: WsStream,
    ack: WsAck,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut write, mut read) = ws.split();
    let mut app = App::new(ack);
    refresh_all(&mut write).await;

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    tick.tick().await; // consume the immediate first tick

    loop {
        terminal.draw(|f| super::ui::render_app(f, &app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            Some(ev) = input_rx.recv() => {
                handle_input(&mut app, ev, &mut write).await;
            }
            msg = read.next(), if app.connected => {
                match msg {
                    Some(Ok(Message::Text(t))) => handle_incoming(&mut app, &t, &mut write).await,
                    Some(Ok(Message::Ping(p))) => { let _ = write.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        app.connected = false;
                        app.status = "disconnected from server".to_string();
                    }
                    _ => {}
                }
            }
            _ = tick.tick(), if app.connected => {
                // Keep uptime and live fields fresh.
                let _ = write.send(client::request(ID_SYSINFO, "system.info", Value::Null)).await;
            }
        }
    }
    Ok(())
}

async fn handle_input(app: &mut App, ev: Event, write: &mut WsWrite) {
    let Event::Key(key) = ev else { return };
    if key.kind != KeyEventKind::Press {
        return;
    }

    if key.code == KeyCode::Char('?') {
        app.show_help = !app.show_help;
        return;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc => app.show_help = false,
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.should_quit = true
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            app.tab = (app.tab + 1) % TABS.len();
            app.selected = 0;
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            app.tab = (app.tab + TABS.len() - 1) % TABS.len();
            app.selected = 0;
        }
        KeyCode::Char(d @ '1'..='6') => {
            app.tab = (d as usize - '1' as usize).min(TABS.len() - 1);
            app.selected = 0;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.current_len();
            if len > 0 {
                app.selected = (app.selected + 1).min(len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected = app.selected.saturating_sub(1);
        }
        KeyCode::Char('r') => {
            app.status = "refreshing…".to_string();
            refresh_all(write).await;
        }
        KeyCode::Enter => toggle_protocol(app, write).await,
        _ => {}
    }
}

/// On the Protocols tab, flip the selected protocol on/off.
async fn toggle_protocol(app: &mut App, write: &mut WsWrite) {
    if app.tab != TAB_PROTOCOLS {
        return;
    }
    let Some(proto) = app.protocols.get(app.selected) else {
        return;
    };
    let Some(name) = proto.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    let enabled = proto
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let method = if enabled {
        "service.protocol.disable"
    } else {
        "service.protocol.enable"
    };
    app.status = format!("{} {name}…", if enabled { "disabling" } else { "enabling" });
    let _ = write
        .send(client::request(
            ID_ACTION,
            method,
            serde_json::json!({ "name": name }),
        ))
        .await;
}

async fn handle_incoming(app: &mut App, text: &str, write: &mut WsWrite) {
    match client::parse_incoming(text) {
        Incoming::Response { id, result } => match result {
            Ok(val) => store_response(app, id, val),
            Err(e) => app.status = e,
        },
        Incoming::Event { collection } => {
            refresh_collection(&collection, write).await;
            app.status = format!("updated: {collection}");
        }
        Incoming::Other(_) => {}
    }
}

fn store_response(app: &mut App, id: i64, val: Value) {
    match id {
        ID_ME => {
            if let Some(u) = val.get("username").and_then(|v| v.as_str())
                && !u.is_empty()
            {
                app.username = u.to_string();
            }
            if let Some(r) = val.get("role").and_then(|v| v.as_str())
                && !r.is_empty()
            {
                app.role = r.to_string();
            }
        }
        ID_SYSINFO => app.system_info = Some(val),
        ID_DEVICES => app.devices = as_array(val),
        ID_FS => app.filesystems = as_array(val),
        ID_SUBVOL => app.subvolumes = as_array(val),
        ID_NFS => app.nfs = as_array(val),
        ID_SMB => app.smb = as_array(val),
        ID_PROTO => app.protocols = as_array(val),
        ID_ACTION => app.status = "done".to_string(),
        _ => {}
    }
    if app.status == "loading…" {
        app.status = "ready".to_string();
    }
    let len = app.current_len();
    if len > 0 && app.selected >= len {
        app.selected = len - 1;
    }
}

async fn refresh_all(write: &mut WsWrite) {
    for (id, method) in [
        (ID_ME, "auth.me"),
        (ID_SYSINFO, "system.info"),
        (ID_DEVICES, "device.list"),
        (ID_FS, "fs.list"),
        (ID_SUBVOL, "subvolume.list_all"),
        (ID_NFS, "share.nfs.list"),
        (ID_SMB, "share.smb.list"),
        (ID_PROTO, "service.protocol.list"),
    ] {
        let _ = write.send(client::request(id, method, Value::Null)).await;
    }
}

async fn refresh_collection(collection: &str, write: &mut WsWrite) {
    let queries: &[(i64, &str)] = match collection {
        "filesystem" => &[(ID_DEVICES, "device.list"), (ID_FS, "fs.list")],
        "subvolume" => &[(ID_SUBVOL, "subvolume.list_all")],
        "share.nfs" => &[(ID_NFS, "share.nfs.list")],
        "share.smb" => &[(ID_SMB, "share.smb.list")],
        "protocol" => &[(ID_PROTO, "service.protocol.list")],
        _ => &[],
    };
    for (id, method) in queries {
        let _ = write.send(client::request(*id, method, Value::Null)).await;
    }
}

fn as_array(val: Value) -> Vec<Value> {
    match val {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}
