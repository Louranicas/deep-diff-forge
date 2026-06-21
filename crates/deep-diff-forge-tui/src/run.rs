use crate::input::map_key;
use crate::state::ReviewApp;
use crate::ui::render;
use crossterm::event::{self, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;

/// Run the interactive review loop against a real terminal.
///
/// This is the one part of the TUI that requires a live TTY and is therefore
/// not unit-tested; all decision logic lives in the tested state model
/// ([`ReviewApp::handle`]) and input mapping ([`map_key`]). The terminal is
/// always restored, even on error.
///
/// # Errors
///
/// Returns any terminal setup, draw, or input I/O error.
pub fn run(mut app: ReviewApp) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut ReviewApp,
) -> io::Result<()> {
    while app.is_running() {
        terminal.draw(|frame| render(frame, app))?;
        if let Event::Key(key) = event::read()? {
            app.handle(map_key(key));
        }
    }
    Ok(())
}
