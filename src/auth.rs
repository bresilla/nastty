//! Minimal username/password auth: users persisted in the nasty state dir,
//! sessions in memory. Wire-compatible with the upstream engine's `Session`
//! shape so the ported router arms work unchanged.

use std::sync::Arc;

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Own state file — never clashes with the upstream engine's auth.json.
const STATE_PATH: &str = "/var/lib/nasty/nastty-auth.json";
const STATE_DIR: &str = "/var/lib/nasty";

/// Login sessions expire after this many seconds.
const SESSION_TTL_SECS: u64 = 8 * 3600; // 8 hours

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    ReadOnly,
    /// Can manage subvolumes, snapshots, shares, and protocol toggles.
    /// Cannot destroy filesystems or manage users.
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    #[serde(default)]
    pub must_change_password: bool,
}

/// Same field set as the upstream engine's `Session` — the router arms
/// read `filesystem` / `owner` for API-token scoping (always None here
/// until token support lands).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub username: String,
    pub role: Role,
    #[serde(default)]
    pub filesystem: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    pub created_at: u64,
    #[serde(default)]
    pub must_change_password: bool,
    #[serde(default)]
    pub client_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthState {
    users: Vec<User>,
    initialized: bool,
}

#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    InvalidToken,
    TokenExpired,
    UserNotFound,
    Internal(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::InvalidCredentials => write!(f, "invalid credentials"),
            AuthError::InvalidToken => write!(f, "invalid token"),
            AuthError::TokenExpired => write!(f, "token expired"),
            AuthError::UserNotFound => write!(f, "user not found"),
            AuthError::Internal(e) => write!(f, "{e}"),
        }
    }
}

pub struct AuthService {
    state: Arc<RwLock<AuthState>>,
    sessions: Arc<RwLock<Vec<Session>>>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    // 16 random bytes for the salt, encoded for SaltString — same
    // approach as the upstream engine.
    let mut salt_bytes = [0u8; 16];
    rand::fill(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| AuthError::Internal(format!("encode salt: {e}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::Internal(format!("hash password: {e}")))
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl AuthService {
    pub async fn new() -> Self {
        let mut state = match tokio::fs::read_to_string(STATE_PATH).await {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warn!("auth state at {STATE_PATH} is unreadable ({e}); starting fresh");
                AuthState::default()
            }),
            Err(_) => AuthState::default(),
        };

        if !state.initialized {
            let hash = hash_password("admin").expect("default password must hash");
            state.users.push(User {
                username: "admin".to_string(),
                password_hash: hash,
                role: Role::Admin,
                must_change_password: true,
            });
            state.initialized = true;
            info!("first run: created default user 'admin' (password 'admin', change required)");
        }

        let service = Self {
            state: Arc::new(RwLock::new(state)),
            sessions: Arc::new(RwLock::new(Vec::new())),
        };
        service.save().await.ok();
        service
    }

    async fn save(&self) -> Result<(), AuthError> {
        let state = self.state.read().await;
        let text = serde_json::to_string_pretty(&*state)
            .map_err(|e| AuthError::Internal(format!("serialize auth state: {e}")))?;
        tokio::fs::create_dir_all(STATE_DIR).await.ok();
        let tmp = format!("{STATE_PATH}.tmp");
        tokio::fs::write(&tmp, text)
            .await
            .map_err(|e| AuthError::Internal(format!("write {tmp}: {e}")))?;
        tokio::fs::rename(&tmp, STATE_PATH)
            .await
            .map_err(|e| AuthError::Internal(format!("rename into {STATE_PATH}: {e}")))
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        client_ip: &str,
    ) -> Result<String, AuthError> {
        let (role, must_change) = {
            let state = self.state.read().await;
            let user = state
                .users
                .iter()
                .find(|u| u.username == username)
                .ok_or(AuthError::InvalidCredentials)?;
            if !verify_password(password, &user.password_hash) {
                return Err(AuthError::InvalidCredentials);
            }
            (user.role.clone(), user.must_change_password)
        };
        let session = Session {
            token: new_token(),
            username: username.to_string(),
            role,
            filesystem: None,
            owner: None,
            created_at: now(),
            must_change_password: must_change,
            client_ip: Some(client_ip.to_string()),
        };
        let token = session.token.clone();
        self.sessions.write().await.push(session);
        Ok(token)
    }

    pub async fn validate(&self, token: &str, _client_ip: &str) -> Result<Session, AuthError> {
        let mut sessions = self.sessions.write().await;
        let idx = sessions
            .iter()
            .position(|s| s.token == token)
            .ok_or(AuthError::InvalidToken)?;
        if now().saturating_sub(sessions[idx].created_at) > SESSION_TTL_SECS {
            sessions.remove(idx);
            return Err(AuthError::TokenExpired);
        }
        Ok(sessions[idx].clone())
    }

    pub async fn logout(&self, token: &str) -> Result<(), AuthError> {
        let mut sessions = self.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|s| s.token != token);
        if sessions.len() == before {
            return Err(AuthError::InvalidToken);
        }
        Ok(())
    }

    pub async fn change_password(
        &self,
        username: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        if new_password.len() < 8 {
            return Err(AuthError::Internal(
                "new password must be at least 8 characters".into(),
            ));
        }
        {
            let mut state = self.state.write().await;
            let user = state
                .users
                .iter_mut()
                .find(|u| u.username == username)
                .ok_or(AuthError::UserNotFound)?;
            if !verify_password(old_password, &user.password_hash) {
                return Err(AuthError::InvalidCredentials);
            }
            user.password_hash = hash_password(new_password)?;
            user.must_change_password = false;
        }
        // Persistence failure is non-fatal (state dir may not exist yet) —
        // the change still applies in memory, same as the upstream engine.
        if let Err(e) = self.save().await {
            warn!("auth state not persisted: {e}");
        }
        // Existing sessions for this user no longer need the change gate.
        for s in self.sessions.write().await.iter_mut() {
            if s.username == username {
                s.must_change_password = false;
            }
        }
        info!("password changed for user '{username}'");
        Ok(())
    }

    // ── user management (admin) ─────────────────────────────────

    pub async fn list_users(&self) -> Vec<UserInfo> {
        self.state
            .read()
            .await
            .users
            .iter()
            .map(|u| UserInfo {
                username: u.username.clone(),
                role: u.role.clone(),
                must_change_password: u.must_change_password,
            })
            .collect()
    }

    pub async fn create_user(
        &self,
        username: &str,
        password: &str,
        role: Role,
    ) -> Result<(), AuthError> {
        if username.is_empty()
            || username.len() > 32
            || !username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AuthError::Internal(
                "username must be 1-32 alphanumeric/'-'/'_' characters".into(),
            ));
        }
        if password.len() < 8 {
            return Err(AuthError::Internal(
                "password must be at least 8 characters".into(),
            ));
        }
        {
            let mut state = self.state.write().await;
            if state.users.iter().any(|u| u.username == username) {
                return Err(AuthError::Internal(format!(
                    "user '{username}' already exists"
                )));
            }
            state.users.push(User {
                username: username.to_string(),
                password_hash: hash_password(password)?,
                role,
                must_change_password: false,
            });
        }
        if let Err(e) = self.save().await {
            warn!("auth state not persisted: {e}");
        }
        info!("created user '{username}'");
        Ok(())
    }

    pub async fn delete_user(&self, actor: &str, username: &str) -> Result<(), AuthError> {
        if actor == username {
            return Err(AuthError::Internal("cannot delete your own account".into()));
        }
        {
            let mut state = self.state.write().await;
            let target = state
                .users
                .iter()
                .find(|u| u.username == username)
                .ok_or(AuthError::UserNotFound)?;
            // Never delete the last admin — that bricks the box.
            if target.role == Role::Admin
                && state.users.iter().filter(|u| u.role == Role::Admin).count() <= 1
            {
                return Err(AuthError::Internal(
                    "cannot delete the last admin account".into(),
                ));
            }
            state.users.retain(|u| u.username != username);
        }
        if let Err(e) = self.save().await {
            warn!("auth state not persisted: {e}");
        }
        // Revoke every session the deleted user had.
        self.sessions
            .write()
            .await
            .retain(|s| s.username != username);
        info!("deleted user '{username}'");
        Ok(())
    }

    /// Admin reset of another user's password: no old password needed,
    /// forces a change on their next login, and revokes their sessions.
    pub async fn admin_set_password(
        &self,
        username: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        if new_password.len() < 8 {
            return Err(AuthError::Internal(
                "new password must be at least 8 characters".into(),
            ));
        }
        {
            let mut state = self.state.write().await;
            let user = state
                .users
                .iter_mut()
                .find(|u| u.username == username)
                .ok_or(AuthError::UserNotFound)?;
            user.password_hash = hash_password(new_password)?;
            user.must_change_password = true;
        }
        if let Err(e) = self.save().await {
            warn!("auth state not persisted: {e}");
        }
        self.sessions
            .write()
            .await
            .retain(|s| s.username != username);
        info!("password reset for user '{username}'");
        Ok(())
    }
}

/// Public listing shape — never exposes password hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
}
