//! Configuration for `nastty serve`. State paths (`/var/lib/nasty`, `/fs`)
//! are compile-time constants inside the upstream nasty crates and are
//! deliberately not configurable here.

use std::net::SocketAddr;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:2137";

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Start even when required tooling (bcachefs) is missing — for
    /// developing the API/TUI on machines that aren't the NAS.
    pub allow_missing_deps: bool,
}

/// Parse arguments that follow the `serve` subcommand.
pub fn parse_serve_args(args: &[String]) -> Result<Config, String> {
    let mut listen: SocketAddr = DEFAULT_LISTEN.parse().expect("default listen must parse");
    let mut allow_missing_deps = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--listen" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--listen requires an address argument".to_string())?;
                listen = value
                    .parse()
                    .map_err(|e| format!("invalid --listen address '{value}': {e}"))?;
            }
            "--allow-missing-deps" => allow_missing_deps = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Config {
        listen,
        allow_missing_deps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_listen_parses() {
        let config = parse_serve_args(&[]).unwrap();
        assert_eq!(config.listen.to_string(), DEFAULT_LISTEN);
    }

    #[test]
    fn listen_override() {
        let config = parse_serve_args(&strings(&["--listen", "127.0.0.1:9999"])).unwrap();
        assert_eq!(config.listen.port(), 9999);
    }

    #[test]
    fn unknown_arg_errors() {
        assert!(parse_serve_args(&strings(&["--bogus"])).is_err());
    }
}
