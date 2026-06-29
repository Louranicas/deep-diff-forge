use crate::input::{map_key, map_mouse};
use crate::state::ReviewApp;
use crate::ui::render;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
use std::time::Duration;

/// RAII guard that restores the terminal on drop — fires on both normal return
/// and panic unwind, so a panic in the event loop cannot leave the user's
/// terminal in raw / alternate-screen mode.
///
/// The guard is *armed* on creation and *disarmed* before the normal-exit
/// cleanup runs, which lets the normal path call `show_cursor()` explicitly
/// (via the `Terminal` handle) while the panic path gets a best-effort restore.
struct TerminalGuard {
    armed: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        Self { armed: true }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort: ignore errors; must not panic inside Drop.
            let _ = disable_raw_mode();
            let _ = crossterm::execute!(
                io::stdout(),
                DisableMouseCapture,
                LeaveAlternateScreen
            );
        }
    }
}

/// Run the interactive review loop against a real terminal.
///
/// This is the one part of the TUI that requires a live TTY and is therefore
/// not unit-tested; all decision logic lives in the tested state model
/// ([`ReviewApp::handle`]) and input mapping ([`map_key`]). The terminal is
/// always restored, even on error or panic.
///
/// # Errors
///
/// Returns any terminal setup, draw, or input I/O error.
pub fn run(mut app: ReviewApp) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut guard = TerminalGuard::new();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = event_loop(&mut terminal, &mut app);

    // Disarm the panic guard and run the normal-path cleanup so we can also
    // call show_cursor() which requires the Terminal handle.
    guard.disarm();
    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut ReviewApp,
) -> io::Result<()> {
    while app.is_running() {
        terminal.draw(|frame| render(frame, app))?;
        // Block for the next event, then drain the entire queued burst before
        // the next redraw. Mouse capture (xterm any-motion mode 1003, enabled
        // above) emits one motion event per cell of travel — hundreds per
        // second. Redrawing once per event pegs a core; coalescing collapses an
        // N-event burst into a single redraw. Blocking on the first `read`
        // keeps an idle review at ~0% CPU.
        dispatch(app, &event::read()?);
        while app.is_running() && event::poll(Duration::ZERO)? {
            dispatch(app, &event::read()?);
        }
    }
    Ok(())
}

/// Apply one terminal event to the app's state machine.
///
/// Mouse-motion and other non-actionable events fold to a no-op so a burst of
/// them costs only the (cheap) `handle` call, not a re-render.
fn dispatch(app: &mut ReviewApp, event: &Event) {
    match event {
        Event::Key(key) => app.handle(map_key(*key)),
        Event::Mouse(mouse) => app.handle(map_mouse(*mouse)),
        _ => {}
    }
}
