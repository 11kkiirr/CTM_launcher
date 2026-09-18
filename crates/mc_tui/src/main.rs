//! CTMLauncher — a terminal Minecraft launcher.

mod app;
mod engine;
mod forms;
mod settings;
mod theme;
mod views;
mod widgets;

use std::io;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

#[tokio::main]
async fn main() -> Result<()> {
    let paths = mc_core::util::Paths::discover()?;
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "ctmlauncher/{} (https://github.com/ctmlauncher)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;

    let mut app = app::App::new(paths, client).await?;

    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Restore the terminal if the application panics.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}
