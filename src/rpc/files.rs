//! RPC arms in the `files.*` domain: a jailed file browser rooted at
//! `/fs`. Every path is canonicalized and verified to stay under the
//! root before any operation — the URL/param is never trusted.

use nasty_common::{Request, Response};
use serde::Serialize;

use super::*;
use crate::auth::{Role, Session};
use crate::state::AppState;

/// Browsing is confined to this root (where filesystems mount).
const ROOT: &str = "/fs";

#[derive(Serialize)]
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    size_bytes: u64,
    modified: u64,
}

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    session: &Session,
) -> Option<Response> {
    let _ = state;
    Some(match req.method.as_str() {
        "files.browse" => {
            let rel = str_param(req, "path").unwrap_or("");
            match browse(rel).await {
                Ok(entries) => ok(req, entries),
                Err(e) => err(req, e),
            }
        }
        "files.mkdir" => {
            if writes_denied(session) {
                return Some(err(req, "read-only role"));
            }
            match (require_str(req, "path"), require_str(req, "name")) {
                (Ok(path), Ok(name)) => match mkdir(path, name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                (Err(r), _) | (_, Err(r)) => r,
            }
        }
        "files.delete" => {
            if writes_denied(session) {
                return Some(err(req, "read-only role"));
            }
            match require_str(req, "path") {
                Ok(path) => match delete(path).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                Err(r) => r,
            }
        }
        "files.rename" => {
            if writes_denied(session) {
                return Some(err(req, "read-only role"));
            }
            match (require_str(req, "path"), require_str(req, "new_name")) {
                (Ok(path), Ok(new_name)) => match rename(path, new_name).await {
                    Ok(()) => ok(req, "ok"),
                    Err(e) => err(req, e),
                },
                (Err(r), _) | (_, Err(r)) => r,
            }
        }
        _ => return None,
    })
}

fn writes_denied(session: &Session) -> bool {
    session.role == Role::ReadOnly
}

/// Resolve a caller-supplied path (absolute like `/fs/tank/x` or relative
/// to the root) to a real path that is provably inside `ROOT`.
fn safe_path(input: &str) -> Result<std::path::PathBuf, String> {
    let root = std::path::Path::new(ROOT);
    let joined = if input.is_empty() || input == "/" {
        root.to_path_buf()
    } else if let Some(stripped) = input.strip_prefix(ROOT) {
        root.join(stripped.trim_start_matches('/'))
    } else if input.starts_with('/') {
        return Err("path must be under /fs".into());
    } else {
        root.join(input)
    };
    // Reject traversal syntactically before touching the filesystem.
    if joined
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("path traversal is not allowed".into());
    }
    Ok(joined)
}

/// Verify a resolved path really is inside the root (defends against
/// symlinks pointing out). Falls back to the lexical path when the target
/// does not exist yet (mkdir/rename destinations).
fn ensure_inside(path: &std::path::Path) -> Result<(), String> {
    let root = std::path::Path::new(ROOT);
    let check = match path.canonicalize() {
        Ok(c) => c,
        Err(_) => path.to_path_buf(),
    };
    if check.starts_with(root) {
        Ok(())
    } else {
        Err("resolved path escapes /fs".into())
    }
}

async fn browse(rel: &str) -> Result<Vec<Entry>, String> {
    let dir = safe_path(rel)?;
    ensure_inside(&dir)?;
    let mut read = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut out = Vec::new();
    while let Ok(Some(entry)) = read.next_entry().await {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size_bytes: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    // Directories first, then alphabetical.
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        Err(format!("invalid name '{name}'"))
    } else {
        Ok(())
    }
}

async fn mkdir(parent: &str, name: &str) -> Result<(), String> {
    valid_name(name)?;
    let dir = safe_path(parent)?.join(name);
    ensure_inside(&dir)?;
    tokio::fs::create_dir(&dir)
        .await
        .map_err(|e| format!("mkdir {}: {e}", dir.display()))
}

async fn delete(target: &str) -> Result<(), String> {
    let path = safe_path(target)?;
    ensure_inside(&path)?;
    // Never delete a filesystem mount root (depth 1 under /fs).
    let rel = path.strip_prefix(ROOT).unwrap_or(&path);
    if rel.components().count() <= 1 {
        return Err("refusing to delete a filesystem root — use the Filesystems tab".into());
    }
    let meta = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|e| format!("stat {}: {e}", path.display()))?;
    if meta.is_dir() {
        tokio::fs::remove_dir_all(&path).await
    } else {
        tokio::fs::remove_file(&path).await
    }
    .map_err(|e| format!("delete {}: {e}", path.display()))
}

async fn rename(target: &str, new_name: &str) -> Result<(), String> {
    valid_name(new_name)?;
    let path = safe_path(target)?;
    ensure_inside(&path)?;
    let dest = path
        .parent()
        .ok_or("cannot rename the root")?
        .join(new_name);
    ensure_inside(&dest)?;
    tokio::fs::rename(&path, &dest)
        .await
        .map_err(|e| format!("rename: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_accepts_within_root() {
        assert_eq!(safe_path("").unwrap(), std::path::Path::new("/fs"));
        assert_eq!(safe_path("tank").unwrap(), std::path::Path::new("/fs/tank"));
        assert_eq!(
            safe_path("/fs/tank/data").unwrap(),
            std::path::Path::new("/fs/tank/data")
        );
    }

    #[test]
    fn safe_path_rejects_escape() {
        assert!(safe_path("../etc").is_err());
        assert!(safe_path("tank/../../etc").is_err());
        assert!(safe_path("/etc/passwd").is_err());
    }

    #[test]
    fn name_validation() {
        assert!(valid_name("photos").is_ok());
        assert!(valid_name("a/b").is_err());
        assert!(valid_name("..").is_err());
        assert!(valid_name("").is_err());
    }
}
