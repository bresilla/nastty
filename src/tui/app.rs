//! Main application: tabbed live views over the NAS state, driven by
//! JSON-RPC responses and server events. Mutations go through modal
//! forms / confirmations built by the action helpers below.

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::client::{self, Incoming, WsAck, WsStream};

use super::Term;

pub(super) type WsWrite = SplitSink<WsStream, Message>;

pub(super) const TABS: [&str; 10] = [
    "Overview",
    "Devices",
    "Filesystems",
    "Subvolumes",
    "Snapshots",
    "Shares",
    "Protocols",
    "Users",
    "Alerts",
    "System",
];
const TAB_DEVICES: usize = 1;
const TAB_FILESYSTEMS: usize = 2;
const TAB_SUBVOLUMES: usize = 3;
const TAB_SNAPSHOTS: usize = 4;
const TAB_SHARES: usize = 5;
const TAB_PROTOCOLS: usize = 6;
const TAB_USERS: usize = 7;
const TAB_ALERTS: usize = 8;
const TAB_SYSTEM: usize = 9;

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
const ID_USERS: i64 = 7;
const ID_SMB_USERS: i64 = 8;
const ID_SMB_GROUPS: i64 = 9;
const ID_ISCSI: i64 = 10;
const ID_NVMEOF: i64 = 11;
const ID_ALERT_RULES: i64 = 12;
const ID_SETTINGS: i64 = 13;
const ID_SSH: i64 = 14;
const ID_STATS: i64 = 102;
const ID_ALERTS: i64 = 103;
const ID_ACTION: i64 = 200;
/// Per-filesystem snapshot queries get ID_SNAP_BASE + index.
const ID_SNAP_BASE: i64 = 300;

// ── modal framework ─────────────────────────────────────────────

pub(super) struct FormField {
    pub label: &'static str,
    pub value: String,
    pub secret: bool,
    /// Select fields cycle options with ←/→ instead of free text.
    pub options: Option<(Vec<String>, usize)>,
    /// Multi-select fields: (item, checked) toggled with space, cursor
    /// moved with ←/→.
    pub multi: Option<(Vec<(String, bool)>, usize)>,
}

impl FormField {
    fn text(label: &'static str, value: &str) -> Self {
        Self {
            label,
            value: value.to_string(),
            secret: false,
            options: None,
            multi: None,
        }
    }
    fn secret(label: &'static str) -> Self {
        Self {
            label,
            value: String::new(),
            secret: true,
            options: None,
            multi: None,
        }
    }
    fn select(label: &'static str, options: Vec<String>, idx: usize) -> Self {
        Self {
            label,
            value: String::new(),
            secret: false,
            options: Some((options, idx)),
            multi: None,
        }
    }
    fn multi(label: &'static str, items: Vec<String>) -> Self {
        Self {
            label,
            value: String::new(),
            secret: false,
            options: None,
            multi: Some((items.into_iter().map(|i| (i, false)).collect(), 0)),
        }
    }
    pub fn display(&self) -> String {
        if let Some((items, cursor)) = &self.multi {
            if items.is_empty() {
                return "(none available)".to_string();
            }
            return items
                .iter()
                .enumerate()
                .map(|(i, (item, checked))| {
                    let mark = if *checked { "▣" } else { "▢" };
                    let short = item.strip_prefix("/dev/").unwrap_or(item);
                    if i == *cursor {
                        format!("[{mark} {short}]")
                    } else {
                        format!(" {mark} {short} ")
                    }
                })
                .collect::<Vec<_>>()
                .join("");
        }
        match &self.options {
            Some((opts, idx)) => format!("◂ {} ▸", opts.get(*idx).cloned().unwrap_or_default()),
            None if self.secret => "•".repeat(self.value.chars().count()),
            None => self.value.clone(),
        }
    }
    fn chosen(&self) -> String {
        if let Some((items, _)) = &self.multi {
            return items
                .iter()
                .filter(|(_, c)| *c)
                .map(|(i, _)| i.clone())
                .collect::<Vec<_>>()
                .join(",");
        }
        match &self.options {
            Some((opts, idx)) => opts.get(*idx).cloned().unwrap_or_default(),
            None => self.value.trim().to_string(),
        }
    }
}

pub(super) struct Form {
    pub title: String,
    pub fields: Vec<FormField>,
    pub focus: usize,
    pub hint: String,
    kind: FormKind,
}

enum FormKind {
    CreateUser,
    ResetPassword {
        username: String,
        is_self: bool,
    },
    CreateSmbUser,
    SetSmbPassword {
        username: String,
    },
    CreateGroup,
    GroupMember {
        add: bool,
    },
    CreateShare,
    CreateFs,
    CreateSubvolume,
    CreateSnapshot,
    CloneSnapshot {
        filesystem: String,
        snapshot: String,
        subvolume: Option<String>,
    },
    CreateAlertRule,
    AddSshKey,
    EditSetting {
        key: &'static str,
    },
}

pub(super) struct Confirm {
    pub message: String,
    /// When set, the user must type this exact string to proceed.
    pub type_to_confirm: Option<String>,
    pub input: String,
    method: &'static str,
    params: Value,
}

pub(super) enum Modal {
    None,
    Form(Form),
    Confirm(Confirm),
}

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
    pub snapshots: Vec<Value>,
    pub nfs: Vec<Value>,
    pub smb: Vec<Value>,
    pub protocols: Vec<Value>,
    pub users: Vec<Value>,
    pub smb_users: Vec<Value>,
    pub smb_groups: Vec<Value>,
    pub iscsi: Vec<Value>,
    pub nvmeof: Vec<Value>,
    pub alerts: Vec<Value>,
    pub alert_rules: Vec<Value>,
    pub settings: Option<Value>,
    pub ssh: Option<Value>,
    pub stats: Option<Value>,
    /// CPU load (percent of cores) history for the dashboard sparkline.
    pub cpu_history: Vec<u64>,
    /// Memory-used percent history for the dashboard sparkline.
    pub mem_history: Vec<u64>,
    pub status: String,
    pub show_help: bool,
    pub modal: Modal,
    /// In-flight snapshot.list request id → filesystem name.
    pending_snapshots: HashMap<i64, String>,
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
            snapshots: Vec::new(),
            nfs: Vec::new(),
            smb: Vec::new(),
            protocols: Vec::new(),
            users: Vec::new(),
            smb_users: Vec::new(),
            smb_groups: Vec::new(),
            iscsi: Vec::new(),
            nvmeof: Vec::new(),
            alerts: Vec::new(),
            alert_rules: Vec::new(),
            settings: None,
            ssh: None,
            stats: None,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            status: "loading…".to_string(),
            show_help: false,
            modal: Modal::None,
            pending_snapshots: HashMap::new(),
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
            TAB_SNAPSHOTS => self.snapshots.len(),
            TAB_SHARES => self.nfs.len() + self.smb.len() + self.iscsi.len() + self.nvmeof.len(),
            TAB_PROTOCOLS => self.protocols.len(),
            TAB_USERS => self.users.len() + self.smb_users.len() + self.smb_groups.len(),
            TAB_ALERTS => self.alert_rules.len(),
            TAB_SYSTEM => self.system_rows().len(),
            _ => 0,
        }
    }

    /// The System tab is a flat list of editable settings + SSH keys.
    pub(super) fn system_rows(&self) -> Vec<SystemRow> {
        let s = self.settings.clone().unwrap_or(Value::Null);
        let sval = |k: &str| {
            s.get(k)
                .map(|v| match v {
                    Value::String(x) => x.clone(),
                    Value::Bool(b) => b.to_string(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "-".into())
        };
        let mut rows = vec![
            SystemRow {
                label: "hostname".into(),
                value: sval("hostname"),
                kind: SystemRowKind::Setting("hostname"),
            },
            SystemRow {
                label: "timezone".into(),
                value: sval("timezone"),
                kind: SystemRowKind::Setting("timezone"),
            },
            SystemRow {
                label: "24h clock".into(),
                value: sval("clock_24h"),
                kind: SystemRowKind::SettingToggle("clock_24h"),
            },
            SystemRow {
                label: "temp unit".into(),
                value: sval("temp_unit"),
                kind: SystemRowKind::Setting("temp_unit"),
            },
        ];
        if let Some(ssh) = &self.ssh {
            let pw = ssh
                .get("password_auth")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            rows.push(SystemRow {
                label: "ssh password auth".into(),
                value: pw.to_string(),
                kind: SystemRowKind::Info,
            });
            for key in ssh
                .get("keys")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let key = key.as_str().unwrap_or_default().to_string();
                let short = key.split_whitespace().last().unwrap_or("key").to_string();
                rows.push(SystemRow {
                    label: format!("ssh key · {short}"),
                    value: format!("{}…", key.chars().take(28).collect::<String>()),
                    kind: SystemRowKind::SshKey(key),
                });
            }
        }
        rows
    }

    /// What the Users-tab selection points at.
    pub(super) fn users_selection(&self) -> UsersSelection {
        let i = self.selected;
        if i < self.users.len() {
            UsersSelection::Account(i)
        } else if i < self.users.len() + self.smb_users.len() {
            UsersSelection::SmbUser(i - self.users.len())
        } else {
            UsersSelection::Group(i - self.users.len() - self.smb_users.len())
        }
    }

    /// What the Shares-tab selection points at.
    pub(super) fn shares_selection(&self) -> SharesSelection {
        let mut i = self.selected;
        if i < self.nfs.len() {
            return SharesSelection::Nfs(i);
        }
        i -= self.nfs.len();
        if i < self.smb.len() {
            return SharesSelection::Smb(i);
        }
        i -= self.smb.len();
        if i < self.iscsi.len() {
            return SharesSelection::Iscsi(i);
        }
        SharesSelection::Nvmeof(i - self.iscsi.len())
    }

    fn fs_names(&self) -> Vec<String> {
        self.filesystems
            .iter()
            .filter_map(|f| f.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect()
    }

    fn free_device_paths(&self) -> Vec<String> {
        self.devices
            .iter()
            .filter(|d| !d.get("in_use").and_then(|v| v.as_bool()).unwrap_or(true))
            .filter_map(|d| d.get("path").and_then(|v| v.as_str()).map(String::from))
            .collect()
    }
}

pub(super) enum UsersSelection {
    Account(usize),
    SmbUser(usize),
    Group(usize),
}

pub(super) enum SharesSelection {
    Nfs(usize),
    Smb(usize),
    Iscsi(usize),
    Nvmeof(usize),
}

pub(super) struct SystemRow {
    pub label: String,
    pub value: String,
    pub kind: SystemRowKind,
}

pub(super) enum SystemRowKind {
    /// Editable free-text setting (system.settings.update key).
    Setting(&'static str),
    /// Boolean setting flipped directly on Enter/e.
    SettingToggle(&'static str),
    /// An authorized SSH key (deletable).
    SshKey(String),
    /// Display-only.
    Info,
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
    refresh_all(&mut app, &mut write).await;

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
                // Keep uptime, live stats, and alerts fresh.
                for (id, method) in [
                    (ID_SYSINFO, "system.info"),
                    (ID_STATS, "system.stats"),
                    (ID_ALERTS, "system.alerts"),
                ] {
                    let _ = write.send(client::request(id, method, Value::Null)).await;
                }
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

    // Modals capture all input first.
    match &mut app.modal {
        Modal::Form(_) => {
            handle_form_key(app, key.code, write).await;
            return;
        }
        Modal::Confirm(_) => {
            handle_confirm_key(app, key.code, write).await;
            return;
        }
        Modal::None => {}
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
        KeyCode::Char(d @ '1'..='9') => {
            app.tab = (d as usize - '1' as usize).min(TABS.len() - 1);
            app.selected = 0;
        }
        KeyCode::Char('0') => {
            app.tab = TABS.len() - 1;
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
            refresh_all(app, write).await;
        }
        KeyCode::Enter => match app.tab {
            TAB_PROTOCOLS => toggle_protocol(app, write).await,
            TAB_ALERTS => toggle_alert_rule(app, write).await,
            TAB_SYSTEM => edit_system_row(app, write).await,
            _ => {}
        },
        KeyCode::Char('n') => open_create_form(app),
        KeyCode::Char('d') => open_delete_confirm(app),
        KeyCode::Char('D') if app.tab == TAB_FILESYSTEMS => open_destroy_confirm(app),
        KeyCode::Char('m') if app.tab == TAB_FILESYSTEMS => mount_toggle(app, write).await,
        KeyCode::Char('s') => match app.tab {
            TAB_FILESYSTEMS => scrub_start(app, write).await,
            TAB_SUBVOLUMES => open_snapshot_form_for_selected(app),
            _ => {}
        },
        KeyCode::Char('e') if app.tab == TAB_SHARES => toggle_share_enabled(app, write).await,
        KeyCode::Char('e') if app.tab == TAB_ALERTS => toggle_alert_rule(app, write).await,
        KeyCode::Char('e') if app.tab == TAB_SYSTEM => edit_system_row(app, write).await,
        KeyCode::Char('c') if app.tab == TAB_SNAPSHOTS => open_clone_snapshot_form(app),
        KeyCode::Char('w') if app.tab == TAB_DEVICES => open_wipe_confirm(app),
        KeyCode::Char('p') if app.tab == TAB_USERS => open_password_form(app),
        KeyCode::Char('g') if app.tab == TAB_USERS => open_group_member_form(app, true),
        KeyCode::Char('G') if app.tab == TAB_USERS => open_group_member_form(app, false),
        _ => {}
    }
}

// ── modal input ─────────────────────────────────────────────────

async fn handle_form_key(app: &mut App, code: KeyCode, write: &mut WsWrite) {
    let Modal::Form(form) = &mut app.modal else {
        return;
    };
    match code {
        KeyCode::Esc => app.modal = Modal::None,
        KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % form.fields.len(),
        KeyCode::BackTab | KeyCode::Up => {
            form.focus = (form.focus + form.fields.len() - 1) % form.fields.len()
        }
        KeyCode::Left | KeyCode::Right => {
            if let Some(field) = form.fields.get_mut(form.focus) {
                if let Some((opts, idx)) = &mut field.options {
                    let n = opts.len().max(1);
                    *idx = if code == KeyCode::Right {
                        (*idx + 1) % n
                    } else {
                        (*idx + n - 1) % n
                    };
                } else if let Some((items, cursor)) = &mut field.multi {
                    let n = items.len().max(1);
                    *cursor = if code == KeyCode::Right {
                        (*cursor + 1) % n
                    } else {
                        (*cursor + n - 1) % n
                    };
                }
            }
        }
        KeyCode::Char(' ') => {
            if let Some(field) = form.fields.get_mut(form.focus) {
                if let Some((items, cursor)) = &mut field.multi {
                    if let Some((_, checked)) = items.get_mut(*cursor) {
                        *checked = !*checked;
                    }
                } else if field.options.is_none() {
                    field.value.push(' ');
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = form.fields.get_mut(form.focus)
                && field.options.is_none()
            {
                field.value.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(field) = form.fields.get_mut(form.focus)
                && field.options.is_none()
            {
                field.value.push(c);
            }
        }
        KeyCode::Enter => match build_request(form) {
            Ok((method, params)) => {
                app.status = format!("{method}…");
                let _ = write.send(client::request(ID_ACTION, method, params)).await;
                app.modal = Modal::None;
            }
            Err(e) => form.hint = e,
        },
        _ => {}
    }
}

async fn handle_confirm_key(app: &mut App, code: KeyCode, write: &mut WsWrite) {
    let Modal::Confirm(confirm) = &mut app.modal else {
        return;
    };
    match code {
        KeyCode::Esc => app.modal = Modal::None,
        KeyCode::Backspace => {
            confirm.input.pop();
        }
        KeyCode::Char('y') if confirm.type_to_confirm.is_none() => {
            let method = confirm.method;
            let params = confirm.params.clone();
            app.status = format!("{method}…");
            let _ = write.send(client::request(ID_ACTION, method, params)).await;
            app.modal = Modal::None;
        }
        KeyCode::Char(c) => {
            if confirm.type_to_confirm.is_some() {
                confirm.input.push(c);
            }
        }
        KeyCode::Enter => {
            let proceed = match &confirm.type_to_confirm {
                Some(expected) => &confirm.input == expected,
                None => true,
            };
            if proceed {
                let method = confirm.method;
                let params = confirm.params.clone();
                app.status = format!("{method}…");
                let _ = write.send(client::request(ID_ACTION, method, params)).await;
                app.modal = Modal::None;
            }
        }
        _ => {}
    }
}

// ── form construction per tab ───────────────────────────────────

fn open_create_form(app: &mut App) {
    let form = match app.tab {
        TAB_FILESYSTEMS => {
            let free = app.free_device_paths();
            let backends: Vec<String> = vec!["btrfs".into(), "bcachefs".into()];
            Form {
                title: "new filesystem / pool".into(),
                hint: "space toggles a device · ◂▸ moves between devices".into(),
                fields: vec![
                    FormField::select("backend", backends, 0),
                    FormField::text("name", "tank"),
                    FormField::multi("devices", free),
                    FormField::select(
                        "raid",
                        vec![
                            "single".into(),
                            "raid0".into(),
                            "raid1".into(),
                            "raid10".into(),
                        ],
                        0,
                    ),
                    FormField::select(
                        "compression",
                        vec!["none".into(), "zstd".into(), "lzo".into(), "lz4".into()],
                        1,
                    ),
                    FormField::text("replicas (bcachefs)", "1"),
                ],
                focus: 1,
                kind: FormKind::CreateFs,
            }
        }
        TAB_SUBVOLUMES => Form {
            title: "new subvolume".into(),
            hint: String::new(),
            fields: vec![
                FormField::select("filesystem", app.fs_names(), 0),
                FormField::text("name", ""),
            ],
            focus: 1,
            kind: FormKind::CreateSubvolume,
        },
        TAB_SNAPSHOTS => Form {
            title: "new snapshot".into(),
            hint: "subvolume as listed on the Subvolumes tab".into(),
            fields: vec![
                FormField::select("filesystem", app.fs_names(), 0),
                FormField::text("subvolume", ""),
                FormField::text("label", "manual"),
            ],
            focus: 1,
            kind: FormKind::CreateSnapshot,
        },
        TAB_SHARES => Form {
            title: "new share".into(),
            hint: "nfs/smb: dir under /fs · iscsi/nvmeof: a block device path".into(),
            fields: vec![
                FormField::select(
                    "kind",
                    vec!["nfs".into(), "smb".into(), "iscsi".into(), "nvmeof".into()],
                    0,
                ),
                FormField::text("name", "share"),
                FormField::text("path / device", "/fs/"),
                FormField::text("comment (nfs/smb)", ""),
                FormField::text("clients (nfs)", "*"),
                FormField::select("read_only", vec!["no".into(), "yes".into()], 0),
                FormField::select("guest_ok (smb)", vec!["no".into(), "yes".into()], 0),
            ],
            focus: 2,
            kind: FormKind::CreateShare,
        },
        TAB_ALERTS => Form {
            title: "new alert rule".into(),
            hint: "threshold is a number (percent, °C, GB…)".into(),
            fields: vec![
                FormField::text("name", ""),
                FormField::select(
                    "metric",
                    vec![
                        "fs_usage_percent".into(),
                        "cpu_load_percent".into(),
                        "memory_usage_percent".into(),
                        "swap_usage_percent".into(),
                        "disk_temperature".into(),
                        "smart_health".into(),
                        "root_disk_free_gb".into(),
                        "boot_disk_free_mb".into(),
                        "kernel_errors".into(),
                    ],
                    0,
                ),
                FormField::select(
                    "condition",
                    vec!["above".into(), "below".into(), "equals".into()],
                    0,
                ),
                FormField::text("threshold", "90"),
                FormField::select("severity", vec!["warning".into(), "critical".into()], 0),
            ],
            focus: 0,
            kind: FormKind::CreateAlertRule,
        },
        TAB_SYSTEM => Form {
            title: "add SSH key".into(),
            hint: "paste a full public key (ssh-ed25519 … / ssh-rsa …)".into(),
            fields: vec![FormField::text("public key", "")],
            focus: 0,
            kind: FormKind::AddSshKey,
        },
        TAB_USERS => match app.users_selection() {
            UsersSelection::Account(_) => Form {
                title: "new account".into(),
                hint: "password min 8 characters".into(),
                fields: vec![
                    FormField::text("username", ""),
                    FormField::secret("password"),
                    FormField::select(
                        "role",
                        vec!["operator".into(), "readonly".into(), "admin".into()],
                        0,
                    ),
                ],
                focus: 0,
                kind: FormKind::CreateUser,
            },
            UsersSelection::SmbUser(_) => Form {
                title: "new SMB user".into(),
                hint: "system user for SMB shares".into(),
                fields: vec![
                    FormField::text("username", ""),
                    FormField::secret("password"),
                ],
                focus: 0,
                kind: FormKind::CreateSmbUser,
            },
            UsersSelection::Group(_) => Form {
                title: "new group".into(),
                hint: String::new(),
                fields: vec![FormField::text("name", "")],
                focus: 0,
                kind: FormKind::CreateGroup,
            },
        },
        _ => return,
    };
    app.modal = Modal::Form(form);
}

fn open_snapshot_form_for_selected(app: &mut App) {
    let Some(sub) = app.subvolumes.get(app.selected) else {
        return;
    };
    let fs = sub
        .get("filesystem")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let name = sub
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    app.modal = Modal::Form(Form {
        title: format!("snapshot {fs}/{name}"),
        hint: String::new(),
        fields: vec![
            FormField::text("filesystem", &fs),
            FormField::text("subvolume", &name),
            FormField::text("label", "manual"),
        ],
        focus: 2,
        kind: FormKind::CreateSnapshot,
    });
}

fn open_clone_snapshot_form(app: &mut App) {
    let Some(snap) = app.snapshots.get(app.selected) else {
        return;
    };
    let filesystem = snap
        .get("filesystem")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let snapshot = snap
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let subvolume = snap
        .get("subvolume")
        .and_then(|v| v.as_str())
        .map(String::from);
    app.modal = Modal::Form(Form {
        title: format!("clone {snapshot}"),
        hint: "creates a writable subvolume from the snapshot".into(),
        fields: vec![FormField::text("new name", "")],
        focus: 0,
        kind: FormKind::CloneSnapshot {
            filesystem,
            snapshot,
            subvolume,
        },
    });
}

fn open_password_form(app: &mut App) {
    match app.users_selection() {
        UsersSelection::Account(i) => {
            let Some(user) = app.users.get(i) else { return };
            let username = user
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let is_self = username == app.username;
            let mut fields = Vec::new();
            if is_self {
                fields.push(FormField::secret("old password"));
            }
            fields.push(FormField::secret("new password"));
            app.modal = Modal::Form(Form {
                title: format!("set password — {username}"),
                hint: if is_self {
                    String::new()
                } else {
                    "user must change it again at next login".into()
                },
                fields,
                focus: 0,
                kind: FormKind::ResetPassword { username, is_self },
            });
        }
        UsersSelection::SmbUser(i) => {
            let Some(user) = app.smb_users.get(i) else {
                return;
            };
            let username = user
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            app.modal = Modal::Form(Form {
                title: format!("set SMB password — {username}"),
                hint: String::new(),
                fields: vec![FormField::secret("password")],
                focus: 0,
                kind: FormKind::SetSmbPassword { username },
            });
        }
        UsersSelection::Group(_) => {}
    }
}

fn open_group_member_form(app: &mut App, add: bool) {
    let user = match app.users_selection() {
        UsersSelection::SmbUser(i) => app
            .smb_users
            .get(i)
            .and_then(|u| u.get("username"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let groups: Vec<String> = app
        .smb_groups
        .iter()
        .filter_map(|g| g.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if groups.is_empty() {
        app.status = "no groups yet — create one with n".into();
        return;
    }
    app.modal = Modal::Form(Form {
        title: if add {
            "add to group".into()
        } else {
            "remove from group".into()
        },
        hint: String::new(),
        fields: vec![
            FormField::text("user", &user),
            FormField::select("group", groups, 0),
        ],
        focus: 0,
        kind: FormKind::GroupMember { add },
    });
}

// ── confirms ────────────────────────────────────────────────────

fn open_delete_confirm(app: &mut App) {
    let confirm = match app.tab {
        TAB_SUBVOLUMES => {
            let Some(s) = app.subvolumes.get(app.selected) else {
                return;
            };
            let fs = str_of(s, "filesystem");
            let name = str_of(s, "name");
            Confirm {
                message: format!("delete subvolume {fs}/{name}?"),
                type_to_confirm: None,
                input: String::new(),
                method: "subvolume.delete",
                params: json!({"filesystem": fs, "name": name}),
            }
        }
        TAB_SNAPSHOTS => {
            let Some(s) = app.snapshots.get(app.selected) else {
                return;
            };
            let fs = str_of(s, "filesystem");
            let name = str_of(s, "name");
            Confirm {
                message: format!("delete snapshot {name} on {fs}?"),
                type_to_confirm: None,
                input: String::new(),
                method: "snapshot.delete",
                params: json!({"filesystem": fs, "name": name}),
            }
        }
        TAB_SHARES => match app.shares_selection() {
            SharesSelection::Nfs(i) => {
                let Some(s) = app.nfs.get(i) else { return };
                let id = str_of(s, "id");
                Confirm {
                    message: format!("delete NFS share {} ({id})?", str_of(s, "path")),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "share.nfs.delete",
                    params: json!({"id": id}),
                }
            }
            SharesSelection::Smb(i) => {
                let Some(s) = app.smb.get(i) else { return };
                let id = str_of(s, "id");
                Confirm {
                    message: format!("delete SMB share {} ({id})?", str_of(s, "name")),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "share.smb.delete",
                    params: json!({"id": id}),
                }
            }
            SharesSelection::Iscsi(i) => {
                let Some(s) = app.iscsi.get(i) else { return };
                let id = str_of(s, "id");
                Confirm {
                    message: format!("delete iSCSI target {} ({id})?", str_of(s, "name")),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "share.iscsi.delete",
                    params: json!({"id": id}),
                }
            }
            SharesSelection::Nvmeof(i) => {
                let Some(s) = app.nvmeof.get(i) else { return };
                let id = str_of(s, "id");
                Confirm {
                    message: format!("delete NVMe-oF subsystem {} ({id})?", str_of(s, "name")),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "share.nvmeof.delete",
                    params: json!({"id": id}),
                }
            }
        },
        TAB_ALERTS => {
            let Some(rule) = app.alert_rules.get(app.selected) else {
                return;
            };
            let id = str_of(rule, "id");
            Confirm {
                message: format!("delete alert rule '{}'?", str_of(rule, "name")),
                type_to_confirm: None,
                input: String::new(),
                method: "alert.rules.delete",
                params: json!({"id": id}),
            }
        }
        TAB_SYSTEM => {
            let rows = app.system_rows();
            let Some(SystemRow {
                kind: SystemRowKind::SshKey(key),
                label,
                ..
            }) = rows.get(app.selected)
            else {
                return;
            };
            Confirm {
                message: format!("remove {label}?"),
                type_to_confirm: None,
                input: String::new(),
                method: "system.ssh.remove_key",
                params: json!({"key": key}),
            }
        }
        TAB_USERS => match app.users_selection() {
            UsersSelection::Account(i) => {
                let Some(u) = app.users.get(i) else { return };
                let name = str_of(u, "username");
                Confirm {
                    message: format!("delete account '{name}'?"),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "auth.delete_user",
                    params: json!({"username": name}),
                }
            }
            UsersSelection::SmbUser(i) => {
                let Some(u) = app.smb_users.get(i) else {
                    return;
                };
                let name = str_of(u, "username");
                Confirm {
                    message: format!("delete SMB user '{name}'?"),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "smb.user.delete",
                    params: json!({"username": name}),
                }
            }
            UsersSelection::Group(i) => {
                let Some(g) = app.smb_groups.get(i) else {
                    return;
                };
                let name = str_of(g, "name");
                Confirm {
                    message: format!("delete group '{name}'?"),
                    type_to_confirm: None,
                    input: String::new(),
                    method: "smb.group.delete",
                    params: json!({"name": name}),
                }
            }
        },
        _ => return,
    };
    app.modal = Modal::Confirm(confirm);
}

fn open_destroy_confirm(app: &mut App) {
    let Some(fs) = app.filesystems.get(app.selected) else {
        return;
    };
    let name = str_of(fs, "name");
    app.modal = Modal::Confirm(Confirm {
        message: format!("DESTROY filesystem '{name}' and wipe its devices?"),
        type_to_confirm: Some(name.clone()),
        input: String::new(),
        method: "fs.destroy",
        params: json!({"name": name}),
    });
}

fn open_wipe_confirm(app: &mut App) {
    let Some(dev) = app.devices.get(app.selected) else {
        return;
    };
    let path = str_of(dev, "path");
    let in_use = dev.get("in_use").and_then(|v| v.as_bool()).unwrap_or(false);
    if in_use {
        app.status = format!("{path} is in use — remove it from its filesystem first");
        return;
    }
    app.modal = Modal::Confirm(Confirm {
        message: format!("WIPE all signatures on {path}?"),
        type_to_confirm: Some(path.clone()),
        input: String::new(),
        method: "device.wipe",
        params: json!({"path": path}),
    });
}

async fn toggle_alert_rule(app: &mut App, write: &mut WsWrite) {
    let Some(rule) = app.alert_rules.get(app.selected) else {
        return;
    };
    let id = str_of(rule, "id");
    let enabled = rule
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    app.status = format!("{} rule…", if enabled { "disabling" } else { "enabling" });
    let _ = write
        .send(client::request(
            ID_ACTION,
            "alert.rules.update",
            json!({"id": id, "enabled": !enabled}),
        ))
        .await;
}

async fn edit_system_row(app: &mut App, write: &mut WsWrite) {
    let rows = app.system_rows();
    let Some(row) = rows.get(app.selected) else {
        return;
    };
    match &row.kind {
        SystemRowKind::Setting(key) => {
            app.modal = Modal::Form(Form {
                title: format!("set {}", row.label),
                hint: String::new(),
                fields: vec![FormField::text("value", &row.value)],
                focus: 0,
                kind: FormKind::EditSetting { key },
            });
        }
        SystemRowKind::SettingToggle(key) => {
            let current = row.value == "true";
            app.status = format!("updating {key}…");
            let _ = write
                .send(client::request(
                    ID_ACTION,
                    "system.settings.update",
                    json!({ *key: !current }),
                ))
                .await;
        }
        SystemRowKind::SshKey(_) | SystemRowKind::Info => {}
    }
}

// ── direct actions ──────────────────────────────────────────────

async fn mount_toggle(app: &mut App, write: &mut WsWrite) {
    let Some(fs) = app.filesystems.get(app.selected) else {
        return;
    };
    let name = str_of(fs, "name");
    let mounted = fs.get("mounted").and_then(|v| v.as_bool()).unwrap_or(false);
    let method = if mounted { "fs.unmount" } else { "fs.mount" };
    app.status = format!("{method} {name}…");
    let _ = write
        .send(client::request(ID_ACTION, method, json!({"name": name})))
        .await;
}

async fn scrub_start(app: &mut App, write: &mut WsWrite) {
    let Some(fs) = app.filesystems.get(app.selected) else {
        return;
    };
    let name = str_of(fs, "name");
    app.status = format!("scrub {name}…");
    let _ = write
        .send(client::request(
            ID_ACTION,
            "fs.scrub.start",
            json!({"name": name}),
        ))
        .await;
}

async fn toggle_share_enabled(app: &mut App, write: &mut WsWrite) {
    let (method, share) = match app.shares_selection() {
        SharesSelection::Nfs(i) => ("share.nfs.update", app.nfs.get(i)),
        SharesSelection::Smb(i) => ("share.smb.update", app.smb.get(i)),
        SharesSelection::Iscsi(_) | SharesSelection::Nvmeof(_) => {
            app.status = "iSCSI/NVMe-oF have no enable toggle — delete/create instead".into();
            return;
        }
    };
    let Some(share) = share else { return };
    let id = str_of(share, "id");
    let enabled = share
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    app.status = format!("{} share…", if enabled { "disabling" } else { "enabling" });
    let _ = write
        .send(client::request(
            ID_ACTION,
            method,
            json!({"id": id, "enabled": !enabled}),
        ))
        .await;
}

/// On the Protocols tab, flip the selected protocol on/off.
async fn toggle_protocol(app: &mut App, write: &mut WsWrite) {
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
        .send(client::request(ID_ACTION, method, json!({ "name": name })))
        .await;
}

// ── request building ────────────────────────────────────────────

fn build_request(form: &mut Form) -> Result<(&'static str, Value), String> {
    let get = |i: usize| -> String { form.fields.get(i).map(|f| f.chosen()).unwrap_or_default() };
    match &form.kind {
        FormKind::CreateUser => {
            require(&get(0), "username")?;
            require(&get(1), "password")?;
            Ok((
                "auth.create_user",
                json!({"username": get(0), "password": get(1), "role": get(2)}),
            ))
        }
        FormKind::ResetPassword { username, is_self } => {
            if *is_self {
                require(&get(0), "old password")?;
                require(&get(1), "new password")?;
                Ok((
                    "auth.change_password",
                    json!({"old_password": get(0), "new_password": get(1)}),
                ))
            } else {
                require(&get(0), "new password")?;
                Ok((
                    "auth.change_password",
                    json!({"username": username, "new_password": get(0)}),
                ))
            }
        }
        FormKind::CreateSmbUser => {
            require(&get(0), "username")?;
            require(&get(1), "password")?;
            Ok((
                "smb.user.create",
                json!({"username": get(0), "password": get(1)}),
            ))
        }
        FormKind::SetSmbPassword { username } => {
            require(&get(0), "password")?;
            Ok((
                "smb.user.set_password",
                json!({"username": username, "password": get(0)}),
            ))
        }
        FormKind::CreateGroup => {
            require(&get(0), "name")?;
            Ok(("smb.group.create", json!({"name": get(0)})))
        }
        FormKind::GroupMember { add } => {
            require(&get(0), "user")?;
            let method = if *add {
                "smb.group.add_member"
            } else {
                "smb.group.remove_member"
            };
            Ok((method, json!({"group": get(1), "user": get(0)})))
        }
        FormKind::CreateShare => {
            require(&get(2), "path / device")?;
            match get(0).as_str() {
                "nfs" => {
                    let host = if get(4).is_empty() {
                        "*".to_string()
                    } else {
                        get(4)
                    };
                    Ok((
                        "share.nfs.create",
                        json!({
                            "path": get(2),
                            "comment": get(3),
                            "clients": [{"host": host, "options": "rw,sync,no_subtree_check"}],
                        }),
                    ))
                }
                "smb" => {
                    require(&get(1), "name")?;
                    Ok((
                        "share.smb.create",
                        json!({
                            "name": get(1),
                            "path": get(2),
                            "comment": get(3),
                            "read_only": get(5) == "yes",
                            "guest_ok": get(6) == "yes",
                        }),
                    ))
                }
                "iscsi" => {
                    require(&get(1), "name")?;
                    Ok((
                        "share.iscsi.create",
                        json!({"name": get(1), "device_path": get(2)}),
                    ))
                }
                _ => {
                    require(&get(1), "name")?;
                    Ok((
                        "share.nvmeof.create",
                        json!({"name": get(1), "device_path": get(2)}),
                    ))
                }
            }
        }
        FormKind::CreateFs => {
            require(&get(1), "name")?;
            let devices: Vec<String> = get(2)
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if devices.is_empty() {
                return Err("select at least one device (space toggles)".into());
            }
            let compression = match get(4).as_str() {
                "none" => None,
                c => Some(c.to_string()),
            };
            if get(0) == "btrfs" {
                Ok((
                    "fs.create",
                    json!({
                        "backend": "btrfs",
                        "name": get(1),
                        "devices": devices,
                        "raid": get(3),
                        "compression": compression,
                    }),
                ))
            } else {
                let replicas: u32 = get(5).parse().unwrap_or(1).max(1);
                let specs: Vec<Value> = devices.iter().map(|d| json!({"path": d})).collect();
                Ok((
                    "fs.create",
                    json!({
                        "name": get(1),
                        "devices": specs,
                        "replicas": replicas,
                        "compression": compression,
                    }),
                ))
            }
        }
        FormKind::CreateSubvolume => {
            require(&get(1), "name")?;
            Ok((
                "subvolume.create",
                json!({
                    "filesystem": get(0),
                    "name": get(1),
                    "subvolume_type": "filesystem",
                }),
            ))
        }
        FormKind::CreateSnapshot => {
            require(&get(1), "subvolume")?;
            require(&get(2), "label")?;
            Ok((
                "snapshot.create",
                json!({
                    "filesystem": get(0),
                    "subvolume": get(1),
                    "name": get(2),
                    "read_only": true,
                }),
            ))
        }
        FormKind::CloneSnapshot {
            filesystem,
            snapshot,
            subvolume,
        } => {
            require(&get(0), "new name")?;
            Ok((
                "snapshot.clone",
                json!({
                    "filesystem": filesystem,
                    "snapshot": snapshot,
                    "subvolume": subvolume,
                    "new_name": get(0),
                }),
            ))
        }
        FormKind::CreateAlertRule => {
            require(&get(0), "name")?;
            let threshold: f64 = get(3)
                .parse()
                .map_err(|_| "threshold must be a number".to_string())?;
            Ok((
                "alert.rules.create",
                json!({
                    "name": get(0),
                    "metric": get(1),
                    "condition": get(2),
                    "threshold": threshold,
                    "severity": get(4),
                    "enabled": true,
                }),
            ))
        }
        FormKind::AddSshKey => {
            require(&get(0), "public key")?;
            Ok(("system.ssh.add_key", json!({"key": get(0)})))
        }
        FormKind::EditSetting { key } => {
            require(&get(0), "value")?;
            Ok(("system.settings.update", json!({ *key: get(0) })))
        }
    }
}

fn require(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} is required"))
    } else {
        Ok(())
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

// ── incoming ────────────────────────────────────────────────────

async fn handle_incoming(app: &mut App, text: &str, write: &mut WsWrite) {
    match client::parse_incoming(text) {
        Incoming::Response { id, result } => match result {
            Ok(val) => store_response(app, id, val, write).await,
            Err(e) => app.status = format!("✗ {e}"),
        },
        Incoming::Event { collection } => {
            refresh_collection(app, &collection, write).await;
            app.status = format!("updated: {collection}");
        }
        Incoming::Other(_) => {}
    }
}

async fn store_response(app: &mut App, id: i64, val: Value, write: &mut WsWrite) {
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
        ID_FS => {
            app.filesystems = as_array(val);
            request_snapshots(app, write).await;
        }
        ID_SUBVOL => app.subvolumes = as_array(val),
        ID_NFS => app.nfs = as_array(val),
        ID_SMB => app.smb = as_array(val),
        ID_PROTO => app.protocols = as_array(val),
        ID_USERS => app.users = as_array(val),
        ID_SMB_USERS => app.smb_users = as_array(val),
        ID_SMB_GROUPS => app.smb_groups = as_array(val),
        ID_ISCSI => app.iscsi = as_array(val),
        ID_NVMEOF => app.nvmeof = as_array(val),
        ID_ALERT_RULES => app.alert_rules = as_array(val),
        ID_ALERTS => app.alerts = as_array(val),
        ID_SETTINGS => app.settings = Some(val),
        ID_SSH => app.ssh = Some(val),
        ID_STATS => {
            // Derive sparkline points: CPU = load1 as % of cores,
            // memory = used as % of total.
            let cores = val
                .pointer("/cpu/count")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as f64;
            let load = val
                .pointer("/cpu/load_1")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            app.cpu_history
                .push(((load / cores) * 100.0).clamp(0.0, 100.0) as u64);
            let total = val
                .pointer("/memory/total_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as f64;
            let used = val
                .pointer("/memory/used_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as f64;
            app.mem_history
                .push(((used / total) * 100.0).clamp(0.0, 100.0) as u64);
            let cap = 120;
            if app.cpu_history.len() > cap {
                app.cpu_history.remove(0);
            }
            if app.mem_history.len() > cap {
                app.mem_history.remove(0);
            }
            app.stats = Some(val);
        }
        ID_ACTION => {
            app.status = "✓ done".to_string();
            // Auth/SMB-account mutations broadcast no event; refresh all.
            refresh_all(app, write).await;
        }
        id if app.pending_snapshots.contains_key(&id) => {
            let fs = app.pending_snapshots.remove(&id).unwrap_or_default();
            let mut snaps = as_array(val);
            for s in &mut snaps {
                if let Some(obj) = s.as_object_mut()
                    && !obj.contains_key("filesystem")
                {
                    obj.insert("filesystem".into(), Value::String(fs.clone()));
                }
            }
            app.snapshots.retain(|s| str_of(s, "filesystem") != fs);
            app.snapshots.extend(snaps);
        }
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

/// Snapshot listings are per-filesystem: fan out one request per mounted
/// filesystem, remembering which id belongs to which name.
async fn request_snapshots(app: &mut App, write: &mut WsWrite) {
    app.pending_snapshots.clear();
    let names: Vec<String> = app
        .filesystems
        .iter()
        .filter(|f| f.get("mounted").and_then(|v| v.as_bool()).unwrap_or(false))
        .filter_map(|f| f.get("name").and_then(|v| v.as_str()).map(String::from))
        .collect();
    if names.is_empty() {
        app.snapshots.clear();
    }
    for (i, name) in names.into_iter().enumerate() {
        let id = ID_SNAP_BASE + i as i64;
        app.pending_snapshots.insert(id, name.clone());
        let _ = write
            .send(client::request(
                id,
                "snapshot.list",
                json!({"filesystem": name}),
            ))
            .await;
    }
}

async fn refresh_all(app: &mut App, write: &mut WsWrite) {
    for (id, method) in [
        (ID_ME, "auth.me"),
        (ID_SYSINFO, "system.info"),
        (ID_DEVICES, "device.list"),
        (ID_FS, "fs.list"),
        (ID_SUBVOL, "subvolume.list_all"),
        (ID_NFS, "share.nfs.list"),
        (ID_SMB, "share.smb.list"),
        (ID_PROTO, "service.protocol.list"),
        (ID_SMB_USERS, "smb.user.list"),
        (ID_SMB_GROUPS, "smb.group.list"),
        (ID_ISCSI, "share.iscsi.list"),
        (ID_NVMEOF, "share.nvmeof.list"),
        (ID_ALERT_RULES, "alert.rules.list"),
        (ID_SETTINGS, "system.settings.get"),
        (ID_SSH, "system.ssh.status"),
        (ID_STATS, "system.stats"),
        (ID_ALERTS, "system.alerts"),
    ] {
        let _ = write.send(client::request(id, method, Value::Null)).await;
    }
    // Admin-only; other roles just get a permission error we ignore.
    if app.role == "admin" {
        let _ = write
            .send(client::request(ID_USERS, "auth.list_users", Value::Null))
            .await;
    }
}

async fn refresh_collection(app: &mut App, collection: &str, write: &mut WsWrite) {
    let queries: &[(i64, &str)] = match collection {
        "filesystem" => &[(ID_DEVICES, "device.list"), (ID_FS, "fs.list")],
        "subvolume" => &[(ID_SUBVOL, "subvolume.list_all")],
        "snapshot" => &[(ID_FS, "fs.list")], // re-fans-out snapshot queries
        "share.nfs" => &[(ID_NFS, "share.nfs.list")],
        "share.smb" => &[(ID_SMB, "share.smb.list")],
        "share.iscsi" => &[(ID_ISCSI, "share.iscsi.list")],
        "share.nvmeof" => &[(ID_NVMEOF, "share.nvmeof.list")],
        "protocol" => &[(ID_PROTO, "service.protocol.list")],
        _ => &[],
    };
    for (id, method) in queries {
        let _ = write.send(client::request(*id, method, Value::Null)).await;
    }
    let _ = app;
}

fn as_array(val: Value) -> Vec<Value> {
    match val {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submit(mut form: Form) -> (String, Value) {
        let (m, p) = build_request(&mut form).expect("build_request");
        (m.to_string(), p)
    }

    #[test]
    fn share_form_builds_nfs_and_smb() {
        let mut app = App::for_test();
        app.tab = TAB_SHARES;
        open_create_form(&mut app);
        let Modal::Form(mut form) = std::mem::replace(&mut app.modal, Modal::None) else {
            panic!("expected form");
        };
        form.fields[2].value = "/fs/tank/data".into();
        let (method, params) = submit(Form {
            fields: form
                .fields
                .iter()
                .map(|f| FormField {
                    label: f.label,
                    value: f.value.clone(),
                    secret: f.secret,
                    options: f.options.clone(),
                    multi: f.multi.clone(),
                })
                .collect(),
            title: form.title.clone(),
            hint: String::new(),
            focus: 0,
            kind: FormKind::CreateShare,
        });
        assert_eq!(method, "share.nfs.create");
        assert_eq!(params["path"], "/fs/tank/data");
        assert_eq!(params["clients"][0]["host"], "*");

        // Flip the kind select to smb.
        if let Some((_, idx)) = &mut form.fields[0].options {
            *idx = 1;
        }
        let (method, params) = submit(form);
        assert_eq!(method, "share.smb.create");
        assert_eq!(params["name"], "share");
        assert_eq!(params["read_only"], false);
    }

    #[test]
    fn fs_form_builds_btrfs_by_default() {
        let mut app = App::for_test();
        app.tab = TAB_FILESYSTEMS;
        app.devices = vec![serde_json::json!({"path":"/dev/sdx","in_use":false})];
        open_create_form(&mut app);
        let Modal::Form(mut form) = std::mem::replace(&mut app.modal, Modal::None) else {
            panic!("expected form");
        };
        // The device multi-select starts unchecked: submitting is refused.
        assert!(build_request(&mut form).is_err());
        if let Some((items, _)) = &mut form.fields[2].multi {
            items[0].1 = true; // space-toggle the first device
        }
        let (method, params) = build_request(&mut form).expect("build");
        assert_eq!(method, "fs.create");
        assert_eq!(params["backend"], "btrfs");
        assert_eq!(params["devices"][0], "/dev/sdx");
        assert_eq!(params["compression"], "zstd");
    }

    #[test]
    fn empty_required_field_is_rejected() {
        let mut form = Form {
            title: "t".into(),
            hint: String::new(),
            fields: vec![
                FormField::text("username", ""),
                FormField::secret("password"),
            ],
            focus: 0,
            kind: FormKind::CreateUser,
        };
        assert!(build_request(&mut form).is_err());
    }
}
