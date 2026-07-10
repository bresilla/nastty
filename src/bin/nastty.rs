//! nastty — terminal UI client for the nasttyd NAS API server.

const DEFAULT_SERVER: &str = "http://127.0.0.1:2137";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("nastty {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "nastty — terminal UI for the nasttyd NAS API server\n\n\
             USAGE: nastty [--server URL] [--user NAME]\n\n\
             OPTIONS:\n\
             \x20 --server URL   server base URL (default {DEFAULT_SERVER})\n\
             \x20 --user NAME    pre-fill the login username\n\
             \x20 -V, --version  print version\n\
             \x20 -h, --help     show this help"
        );
        return Ok(());
    }

    let mut server = DEFAULT_SERVER.to_string();
    let mut user: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--server" => {
                server = iter
                    .next()
                    .ok_or("--server requires a URL argument")?
                    .clone();
            }
            "--user" => {
                user = Some(
                    iter.next()
                        .ok_or("--user requires a name argument")?
                        .clone(),
                );
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    nastty::tui::run(server, user).await
}
