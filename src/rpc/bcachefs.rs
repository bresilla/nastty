//! Read-only passthroughs for bcachefs diagnostics.

use nasty_common::{Request, Response};

use super::*;
use crate::state::AppState;

pub(super) async fn try_route(req: &Request, state: &AppState) -> Option<Response> {
    Some(match req.method.as_str() {
        "bcachefs.usage" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.bcachefs_usage(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "bcachefs.top" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.bcachefs_top(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "bcachefs.timestats" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.bcachefs_timestats(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        _ => return None,
    })
}
