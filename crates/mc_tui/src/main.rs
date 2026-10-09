//! CTMLauncher — a terminal Minecraft launcher.

mod app;
mod edit;
mod engine;
mod forms;
mod i18n;
mod images;
mod md;
mod settings;
mod theme;
mod views;
mod widgets;
mod wizard;

use std::io;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
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
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;

    // Query the terminal for an image graphics protocol (kitty/sixel/iTerm2)
    // and its font size. The query briefly toggles raw mode off internally, so
    // re-enable it afterwards. Fall back to half-blocks when the query fails or
    // times out, so images still render on plain terminals.
    let queried = match ratatui_image::picker::Picker::from_query_stdio() {
        Ok(picker) => Some(picker),
        Err(err) => {
            if std::env::var_os("CTM_IMAGE_LOG").is_some() {
                eprintln!("[images] capability query failed: {err}; falling back to half-blocks");
            }
            None
        }
    };
    let picker = queried.unwrap_or_else(ratatui_image::picker::Picker::halfblocks);
    if std::env::var_os("CTM_IMAGE_LOG").is_some() {
        eprintln!(
            "[images] protocol={:?} font={:?} caps={:?} TERM={:?} kitty_env={} wezterm_env={}",
            picker.protocol_type(),
            picker.font_size(),
            picker.capabilities(),
            std::env::var("TERM").ok(),
            std::env::var_os("KITTY_WINDOW_ID").is_some(),
            std::env::var_os("WEZTERM_EXECUTABLE").is_some(),
        );
    }
    app.picker = picker;
    enable_raw_mode()?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = app.run(&mut terminal).await;
    app.shutdown_presence();

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}

/// Restore the terminal if the application panics.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
        original(info);
    }));
}
