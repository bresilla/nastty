//! Single nastty executable: `nastty tui` and `nastty serve`.

const DEFAULT_SERVER: &str = "http://127.0.0.1:2137";

enum Action {
    Tui {
        server: String,
        user: Option<String>,
    },
    Serve(nastty::config::Config),
    Help(Help),
    Version,
}

enum Help {
    Main,
    Tui,
    Serve,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let action = match parse(&args) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("error: {error}\nTry `nastty --help`.");
            std::process::exit(2);
        }
    };
    match action {
        Action::Tui { server, user } => nastty::tui::run(server, user).await,
        Action::Serve(config) => nastty::serve::run(config).await,
        Action::Help(help) => {
            print_help(help);
            Ok(())
        }
        Action::Version => {
            println!("nastty {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn parse(args: &[String]) -> Result<Action, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Action::Help(Help::Main));
    };
    match command {
        "-h" | "--help" | "help" => Ok(Action::Help(Help::Main)),
        "-V" | "--version" => Ok(Action::Version),
        "tui" => parse_tui(&args[1..]),
        "serve" => parse_serve(&args[1..]),
        other => Err(format!(
            "unknown command '{other}'; expected `tui` or `serve`"
        )),
    }
}

fn parse_tui(args: &[String]) -> Result<Action, String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return Ok(Action::Help(Help::Tui));
    }
    let mut server = DEFAULT_SERVER.to_string();
    let mut user = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => {
                server = args
                    .next()
                    .ok_or_else(|| "--server requires a URL".to_string())?
                    .clone();
            }
            "--user" => {
                user = Some(
                    args.next()
                        .ok_or_else(|| "--user requires a name".to_string())?
                        .clone(),
                );
            }
            other => return Err(format!("unknown tui option '{other}'")),
        }
    }
    Ok(Action::Tui { server, user })
}

fn parse_serve(args: &[String]) -> Result<Action, String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return Ok(Action::Help(Help::Serve));
    }
    nastty::config::parse_serve_args(args).map(Action::Serve)
}

fn print_help(help: Help) {
    match help {
        Help::Main => println!(
            "nastty — bcachefs NAS server and terminal interface\n\n\
             USAGE:\n\
             \x20 nastty <COMMAND> [OPTIONS]\n\n\
             COMMANDS:\n\
             \x20 tui      Open the terminal interface\n\
             \x20 serve    Run the NAS API server and metrics collector\n\n\
             OPTIONS:\n\
             \x20 -V, --version    Print version\n\
             \x20 -h, --help       Show this help\n\n\
             Run `nastty tui --help` or `nastty serve --help` for command options."
        ),
        Help::Tui => println!(
            "Open the nastty terminal interface\n\n\
             USAGE: nastty tui [OPTIONS]\n\n\
             OPTIONS:\n\
             \x20 --server URL    Server base URL (default {DEFAULT_SERVER})\n\
             \x20 --user NAME     Pre-fill the login username\n\
             \x20 -h, --help      Show this help"
        ),
        Help::Serve => println!(
            "Run the NAS API server and built-in metrics collector\n\n\
             USAGE: nastty serve [OPTIONS]\n\n\
             OPTIONS:\n\
             \x20 --listen ADDR         Listen address (default {})\n\
             \x20 --allow-missing-deps  Run without bcachefs for development\n\
             \x20 -h, --help            Show this help",
            nastty::config::DEFAULT_LISTEN
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn requires_one_of_two_subcommands() {
        assert!(matches!(parse(&[]).unwrap(), Action::Help(Help::Main)));
        assert!(parse(&strings(&["daemon"])).is_err());
    }

    #[test]
    fn parses_tui_subcommand() {
        let Action::Tui { server, user } = parse(&strings(&[
            "tui",
            "--server",
            "http://nas:2137",
            "--user",
            "admin",
        ]))
        .unwrap() else {
            panic!("expected tui action");
        };
        assert_eq!(server, "http://nas:2137");
        assert_eq!(user.as_deref(), Some("admin"));
    }

    #[test]
    fn parses_serve_subcommand() {
        let Action::Serve(config) = parse(&strings(&[
            "serve",
            "--listen",
            "127.0.0.1:9999",
            "--allow-missing-deps",
        ]))
        .unwrap() else {
            panic!("expected serve action");
        };
        assert_eq!(config.listen.port(), 9999);
        assert!(config.allow_missing_deps);
    }
}
