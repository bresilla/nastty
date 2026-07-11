//! Terminal UI for `nastty serve`. Logs in, opens the JSON-RPC
//! WebSocket, renders live NAS state, and refreshes on server events.

mod app;
mod login;
mod theme;
mod ui;

use std::io::{Stdout, stdout};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use tokio::sync::mpsc;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// Run the TUI against `base` (e.g. `http://127.0.0.1:2137`), pre-filling
/// the login username with `user` when given.
pub async fn run(base: String, user: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = setup_terminal()?;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Event>();
    let input_stop = spawn_input_thread(input_tx);

    let result = drive(&mut terminal, &mut input_rx, &base, user).await;

    input_stop();
    restore_terminal(&mut terminal)?;
    result
}

async fn drive(
    terminal: &mut Term,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
    base: &str,
    user: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // `None` means the user quit at the login screen.
    if let Some((ws, ack)) = login::login_flow(terminal, input_rx, base, user).await? {
        app::run_app(terminal, input_rx, ws, ack).await?;
    }
    Ok(())
}

fn setup_terminal() -> Result<Term, Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    // Restore the terminal even if the app panics, so the user isn't left
    // in raw mode with a garbled screen.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        default_hook(info);
    }));
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn restore_terminal(terminal: &mut Term) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Spawn a thread that reads terminal input and forwards it. Returns a
/// closure that stops the thread.
fn spawn_input_thread(tx: mpsc::UnboundedSender<Event>) -> impl FnOnce() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let running = Arc::new(AtomicBool::new(true));
    let thread_flag = running.clone();
    let handle = std::thread::spawn(move || {
        while thread_flag.load(Ordering::Relaxed) {
            // Poll with a timeout so the stop flag is checked regularly.
            match event::poll(Duration::from_millis(150)) {
                Ok(true) => match event::read() {
                    Ok(ev) => {
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    });

    move || {
        running.store(false, Ordering::Relaxed);
        let _ = handle.join();
    }
}
