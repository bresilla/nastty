//! CLI configuration for nasttyd. State paths (`/var/lib/nasty`, `/fs`)
//! are compile-time constants inside the upstream nasty crates and are
//! deliberately not configurable here.

use std::net::SocketAddr;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:2137";

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
}

pub enum CliAction {
    Run(Config),
    Exit,
}

/// Hand-rolled arg parsing, same style as the upstream engine.
pub fn parse_args(args: &[String]) -> Result<CliAction, String> {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("nasttyd {}", env!("CARGO_PKG_VERSION"));
        return Ok(CliAction::Exit);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "nasttyd — thin local NAS API server built on the nasty crates\n\n\
             USAGE: nasttyd [--listen ADDR]\n\n\
             OPTIONS:\n\
             \x20 --listen ADDR   listen address (default {DEFAULT_LISTEN})\n\
             \x20 -V, --version   print version\n\
             \x20 -h, --help      show this help"
        );
        return Ok(CliAction::Exit);
    }

    let mut listen: SocketAddr = DEFAULT_LISTEN.parse().expect("default listen must parse");
    let mut iter = args.iter().skip(1);
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
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(CliAction::Run(Config { listen }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_listen_parses() {
        match parse_args(&strings(&["nasttyd"])).unwrap() {
            CliAction::Run(c) => assert_eq!(c.listen.to_string(), DEFAULT_LISTEN),
            CliAction::Exit => panic!("expected run"),
        }
    }

    #[test]
    fn listen_override() {
        match parse_args(&strings(&["nasttyd", "--listen", "127.0.0.1:9999"])).unwrap() {
            CliAction::Run(c) => assert_eq!(c.listen.port(), 9999),
            CliAction::Exit => panic!("expected run"),
        }
    }

    #[test]
    fn unknown_arg_errors() {
        assert!(parse_args(&strings(&["nasttyd", "--bogus"])).is_err());
    }
}
