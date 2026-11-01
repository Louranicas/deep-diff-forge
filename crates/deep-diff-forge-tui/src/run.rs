use crate::input::{map_key, map_mouse};
use crate::state::ReviewApp;
use crate::ui::render;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use std::io::{self, Write};
use std::time::Duration;

/// The terminal device a live review takes over: where its mode escape sequences go, how raw
/// mode is switched, and where input events come from.
///
/// [`run`] passes stdout and crossterm's own functions, so production has no adapter code of its
/// own. Tests pass a recording sink and a scripted queue, and so drive the same [`session`] and
/// [`event_loop`] the binary runs.
struct Tty<W: Write> {
    out: W,
    raw_on: fn() -> io::Result<()>,
    raw_off: fn() -> io::Result<()>,
    read: fn() -> io::Result<Event>,
    poll: fn(Duration) -> io::Result<bool>,
}

impl<W: Write> Tty<W> {
    /// Leave raw mode, mouse capture and the alternate screen. Both halves are attempted even
    /// when the first fails; the first error is returned.
    fn restore(&mut self) -> io::Result<()> {
        let raw = (self.raw_off)();
        let screen = crossterm::execute!(self.out, DisableMouseCapture, LeaveAlternateScreen);
        raw.and(screen)
    }
}

/// RAII guard that restores the terminal on drop — fires on both normal return
/// and panic unwind, so a panic in the event loop cannot leave the user's
/// terminal in raw / alternate-screen mode.
///
/// The guard is *armed* once raw mode is on and *disarmed* by [`TerminalGuard::finish`], which
/// lets the normal path report a failed restore while the panic path gets a best-effort one.
struct TerminalGuard<W: Write> {
    tty: Tty<W>,
    armed: bool,
}

impl<W: Write> TerminalGuard<W> {
    /// The normal-path restore: disarm, then restore and report its error.
    fn finish(&mut self) -> io::Result<()> {
        self.armed = false;
        self.tty.restore()
    }
}

impl<W: Write> Drop for TerminalGuard<W> {
    fn drop(&mut self) {
        if self.armed {
            // Best-effort: ignore errors; must not panic inside Drop.
            let _ = self.tty.restore();
        }
    }
}

/// Run the interactive review loop against a real terminal.
///
/// The loop and the terminal restore behind this function are tested headlessly, driven by
/// scripted events against a `TestBackend`; the CLI's `review_tty` test runs this function
/// itself in a pseudo-terminal. The terminal is always restored, even on error or panic.
///
/// # Errors
///
/// Returns any terminal setup, draw, or input I/O error.
pub fn run(mut app: ReviewApp) -> io::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let tty = Tty {
        out: io::stdout(),
        raw_on: enable_raw_mode,
        raw_off: disable_raw_mode,
        read: event::read,
        poll: event::poll,
    };
    session(&mut terminal, tty, &mut app)
}

/// Take over the terminal, run the loop, and give the terminal back.
///
/// Raw mode is switched on first; if that fails nothing has changed and nothing is written. From
/// then on the guard is armed, so a failure entering the alternate screen, an error in the loop
/// or a panic all leave raw mode, mouse capture and the alternate screen. The loop's own error is
/// returned unless the restore itself fails.
fn session<B, W>(terminal: &mut Terminal<B>, tty: Tty<W>, app: &mut ReviewApp) -> io::Result<()>
where
    B: Backend,
    io::Error: From<B::Error>,
    W: Write,
{
    (tty.raw_on)()?;
    let mut guard = TerminalGuard { tty, armed: true };
    crossterm::execute!(guard.tty.out, EnterAlternateScreen, EnableMouseCapture)?;

    let result = event_loop(terminal, app, guard.tty.read, guard.tty.poll);

    guard.finish()?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut ReviewApp,
    read: fn() -> io::Result<Event>,
    poll: fn(Duration) -> io::Result<bool>,
) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    while app.is_running() {
        terminal.draw(|frame| render(frame, app))?;
        // Block for the next event, then drain the entire queued burst before
        // the next redraw. Mouse capture (xterm any-motion mode 1003, enabled
        // above) emits one motion event per cell of travel — hundreds per
        // second. Redrawing once per event pegs a core; coalescing collapses an
        // N-event burst into a single redraw. Blocking on the first `read`
        // keeps an idle review at ~0% CPU.
        dispatch(app, &read()?);
        while app.is_running() && poll(Duration::ZERO)? {
            dispatch(app, &read()?);
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

#[cfg(test)]
mod tests {
    //! The live loop, driven the way a terminal drives it: a scripted input queue, a recording
    //! raw-mode switch and escape-sequence sink, and a `TestBackend` screen. Every test runs the
    //! production `session`, so what is asserted here is what `run` does with a real terminal.

    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use deep_diff_forge_patch::parse;
    use ratatui::backend::{ClearType, TestBackend, WindowSize};
    use ratatui::buffer::{Buffer, Cell};
    use ratatui::layout::{Position, Size};
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const PATCH: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
 fn keep() {}
-let old = 1;
+let new = 2;
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-a
+b
";

    /// One scripted input step, consumed by the fake `read`.
    enum Step {
        /// An event that arrives after the screen has been redrawn: `poll` reports an empty queue
        /// in front of it.
        Later(Event),
        /// An event already queued behind the previous one, in the same burst.
        Queued(Event),
        /// The terminal is resized, then reports it (crossterm's order).
        Resize(u16, u16),
        /// `read` fails.
        ReadFails,
        /// `poll` fails.
        PollFails,
        /// `read` panics, standing in for a bug anywhere in the loop.
        Panic,
    }

    /// Calls past which the fake input refuses, so a loop that stopped reading or polling
    /// properly ends with an error instead of hanging the test run.
    const MAX_CALLS: usize = 1_000;

    /// The one device operation a test makes fail.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        RawOn,
        RawOff,
        Write,
        Draw,
    }

    /// The fake terminal device. `Tty` takes plain function pointers, which cannot capture, so the
    /// device is thread-local: each test runs on its own thread, and [`Device::install`] resets it.
    struct Device {
        script: VecDeque<Step>,
        screen: TestBackend,
        raw: bool,
        fault: Fault,
        written: Vec<u8>,
        calls: usize,
    }

    thread_local! {
        static DEVICE: RefCell<Device> = RefCell::new(Device::new(Vec::new()));
    }

    impl Device {
        fn new(script: Vec<Step>) -> Self {
            Self {
                script: script.into(),
                screen: TestBackend::new(100, 30),
                raw: false,
                fault: Fault::None,
                written: Vec::new(),
                calls: 0,
            }
        }

        fn install(script: Vec<Step>) {
            DEVICE.with(|d| *d.borrow_mut() = Self::new(script));
        }

        fn with<T>(f: impl FnOnce(&mut Self) -> T) -> T {
            DEVICE.with(|d| f(&mut d.borrow_mut()))
        }

        fn count_call(&mut self) -> io::Result<()> {
            self.calls += 1;
            if self.calls > MAX_CALLS {
                return Err(io::Error::other(
                    "the loop kept calling the input past its bound",
                ));
            }
            Ok(())
        }
    }

    fn raw_on() -> io::Result<()> {
        Device::with(|d| {
            if d.fault == Fault::RawOn {
                return Err(io::Error::other("raw mode refused"));
            }
            d.raw = true;
            Ok(())
        })
    }

    fn raw_off() -> io::Result<()> {
        Device::with(|d| {
            d.raw = false;
            if d.fault == Fault::RawOff {
                return Err(io::Error::other("raw mode stuck"));
            }
            Ok(())
        })
    }

    #[expect(
        clippy::panic_in_result_fn,
        reason = "Step::Panic stands in for a bug anywhere in the loop"
    )]
    fn read() -> io::Result<Event> {
        let step = Device::with(|d| {
            d.count_call()?;
            Ok::<_, io::Error>(d.script.pop_front())
        })?;
        match step {
            Some(Step::Later(event) | Step::Queued(event)) => Ok(event),
            Some(Step::Resize(width, height)) => {
                Device::with(|d| d.screen.resize(width, height));
                Ok(Event::Resize(width, height))
            }
            Some(Step::ReadFails | Step::PollFails) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the input device failed",
            )),
            Some(Step::Panic) => panic!("a bug in the loop"),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "scripted input ended",
            )),
        }
    }

    fn poll(timeout: Duration) -> io::Result<bool> {
        Device::with(|d| {
            d.count_call()?;
            if timeout != Duration::ZERO {
                return Err(io::Error::other("the drain waited"));
            }
            match d.script.front() {
                Some(Step::Queued(_)) => Ok(true),
                Some(Step::PollFails) => Err(io::Error::other("poll failed")),
                _ => Ok(false),
            }
        })
    }

    /// Escape sequences sent to the terminal, recorded on the device.
    struct Sink;

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Device::with(|d| {
                if d.fault == Fault::Write {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stdout closed"));
                }
                d.written.extend_from_slice(bytes);
                Ok(bytes.len())
            })
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The device's screen as a ratatui backend with crossterm's `io::Error`.
    struct Screen;

    /// A `TestBackend` result (which cannot fail) as crossterm's `io::Result`.
    fn lift<T>(result: Result<T, std::convert::Infallible>) -> io::Result<T> {
        result.map_err(|never| match never {})
    }

    impl Backend for Screen {
        type Error = io::Error;
        fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            Device::with(|d| {
                if d.fault == Fault::Draw {
                    return Err(io::Error::other("draw failed"));
                }
                lift(d.screen.draw(content))
            })
        }
        fn hide_cursor(&mut self) -> io::Result<()> {
            Device::with(|d| lift(d.screen.hide_cursor()))
        }
        fn show_cursor(&mut self) -> io::Result<()> {
            Device::with(|d| lift(d.screen.show_cursor()))
        }
        fn get_cursor_position(&mut self) -> io::Result<Position> {
            Device::with(|d| lift(d.screen.get_cursor_position()))
        }
        fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
            Device::with(|d| lift(d.screen.set_cursor_position(position)))
        }
        fn clear(&mut self) -> io::Result<()> {
            Device::with(|d| lift(d.screen.clear()))
        }
        fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
            Device::with(|d| lift(d.screen.clear_region(clear_type)))
        }
        fn size(&self) -> io::Result<Size> {
            Device::with(|d| lift(d.screen.size()))
        }
        fn window_size(&mut self) -> io::Result<WindowSize> {
            Device::with(|d| lift(d.screen.window_size()))
        }
        fn flush(&mut self) -> io::Result<()> {
            Device::with(|d| lift(Backend::flush(&mut d.screen)))
        }
    }

    fn app() -> ReviewApp {
        let files = parse(PATCH).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        ReviewApp::from_review_with_annotations(&files, notes)
    }

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn tty() -> Tty<Sink> {
        Tty {
            out: Sink,
            raw_on,
            raw_off,
            read,
            poll,
        }
    }

    /// Entering: the alternate screen (`ESC [?1049h`), then mouse capture.
    fn entered() -> Vec<u8> {
        let mut bytes = Vec::new();
        crossterm::execute!(bytes, EnterAlternateScreen, EnableMouseCapture).unwrap();
        assert!(bytes.starts_with(b"\x1b[?1049h"), "{bytes:?}");
        bytes
    }

    /// Restoring: mouse capture off, then the alternate screen left (`ESC [?1049l`).
    fn restored() -> Vec<u8> {
        let mut bytes = Vec::new();
        crossterm::execute!(bytes, DisableMouseCapture, LeaveAlternateScreen).unwrap();
        assert!(bytes.ends_with(b"\x1b[?1049l"), "{bytes:?}");
        bytes
    }

    /// What one live review did: the session's result, the frames drawn, the final screen.
    struct Outcome {
        result: io::Result<()>,
        frames: usize,
        screen: Buffer,
        cursor_visible: bool,
    }

    fn review(app: &mut ReviewApp, script: Vec<Step>, fault: Fault) -> Outcome {
        Device::install(script);
        Device::with(|d| d.fault = fault);
        let mut terminal = Terminal::new(Screen).unwrap();
        let result = session(&mut terminal, tty(), app);
        Outcome {
            result,
            frames: terminal.get_frame().count(),
            screen: Device::with(|d| d.screen.buffer().clone()),
            cursor_visible: Device::with(|d| d.screen.cursor_visible()),
        }
    }

    /// The frame the headless renderer draws for `app` at this size.
    fn expected_frame(app: &ReviewApp, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The device was restored: raw mode off and the restore sequence written last.
    fn assert_restored() {
        Device::with(|d| {
            assert!(!d.raw, "raw mode left on");
            assert!(
                d.written.ends_with(&restored()),
                "no restore sequence at the end: {:?}",
                String::from_utf8_lossy(&d.written)
            );
        });
    }

    #[test]
    fn a_key_moves_the_review_and_q_quits_with_the_terminal_restored() {
        let mut live = app();
        let out = review(
            &mut live,
            vec![Step::Later(key('j')), Step::Later(key('q'))],
            Fault::None,
        );
        out.result.unwrap();
        assert!(!live.is_running());
        assert_eq!(live.selected_index(), 1, "j selected the second file");
        // One frame before each blocking read; the quit is not drawn.
        assert_eq!(out.frames, 2);
        let mut moved = app();
        moved.handle(crate::state::AppEvent::Next);
        assert_eq!(
            out.screen,
            expected_frame(&moved, 100, 30),
            "the live frame is the render"
        );
        assert!(out.cursor_visible, "the cursor is shown again");
        let written = Device::with(|d| d.written.clone());
        assert_eq!(written, [entered(), restored()].concat());
        assert_restored();
    }

    #[test]
    fn a_queued_burst_is_applied_before_one_redraw() {
        let mut live = app();
        let script = vec![
            Step::Later(key('j')),
            Step::Queued(key('k')),
            Step::Queued(key('j')),
            Step::Later(key('q')),
        ];
        let out = review(&mut live, script, Fault::None);
        out.result.unwrap();
        assert_eq!(live.selected_index(), 1);
        assert_eq!(out.frames, 2, "three queued keys, one redraw");
    }

    #[test]
    fn events_queued_after_quit_are_not_read() {
        let mut live = app();
        let out = review(
            &mut live,
            vec![Step::Later(key('q')), Step::Queued(key('j'))],
            Fault::None,
        );
        out.result.unwrap();
        assert_eq!(live.selected_index(), 0, "the key after q was applied");
        assert_eq!(
            Device::with(|d| d.script.len()),
            1,
            "the key after q was read"
        );
        assert_eq!(out.frames, 1);
    }

    #[test]
    fn a_resize_redraws_at_the_new_size() {
        let mut live = app();
        let out = review(
            &mut live,
            vec![Step::Resize(60, 20), Step::Later(key('q'))],
            Fault::None,
        );
        out.result.unwrap();
        assert_eq!(out.frames, 2);
        assert_eq!(out.screen, expected_frame(&app(), 60, 20));
    }

    #[test]
    fn a_mouse_click_reaches_the_review() {
        let mut live = app();
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 70,
            row: 10,
            modifiers: KeyModifiers::NONE,
        });
        let before = live.focus();
        let out = review(
            &mut live,
            vec![Step::Later(click), Step::Later(key('q'))],
            Fault::None,
        );
        out.result.unwrap();
        assert_ne!(
            live.focus(),
            before,
            "a click in the diff pane moves focus there"
        );
    }

    #[test]
    fn input_that_ends_returns_its_error_and_restores_the_terminal() {
        let mut live = app();
        let out = review(&mut live, vec![Step::Later(key('j'))], Fault::None);
        let error = out.result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), "scripted input ended");
        assert_eq!(live.selected_index(), 1, "input before the end was applied");
        assert!(out.cursor_visible);
        assert_restored();
    }

    #[test]
    fn a_failing_read_returns_its_error_and_restores_the_terminal() {
        let out = review(&mut app(), vec![Step::ReadFails], Fault::None);
        let error = out.result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "the input device failed");
        assert_restored();
    }

    #[test]
    fn a_failing_poll_returns_its_error_and_restores_the_terminal() {
        let out = review(
            &mut app(),
            vec![Step::Later(key('j')), Step::PollFails],
            Fault::None,
        );
        assert_eq!(out.result.unwrap_err().to_string(), "poll failed");
        assert_restored();
    }

    #[test]
    fn a_failing_draw_returns_its_error_and_restores_the_terminal() {
        let out = review(&mut app(), vec![Step::Later(key('q'))], Fault::Draw);
        assert_eq!(out.result.unwrap_err().to_string(), "draw failed");
        assert_eq!(
            Device::with(|d| d.script.len()),
            1,
            "no input is read without a frame"
        );
        assert_restored();
    }

    #[test]
    fn a_panic_in_the_loop_restores_the_terminal() {
        Device::install(vec![Step::Panic]);
        let mut terminal = Terminal::new(Screen).unwrap();
        let mut live = app();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session(&mut terminal, tty(), &mut live)
        }));
        assert!(unwound.is_err(), "the panic propagates after the restore");
        assert_restored();
    }

    #[test]
    fn a_failed_restore_is_reported_after_a_clean_quit() {
        let out = review(&mut app(), vec![Step::Later(key('q'))], Fault::RawOff);
        assert_eq!(out.result.unwrap_err().to_string(), "raw mode stuck");
        // The screen half of the restore still ran.
        assert!(Device::with(|d| d.written.ends_with(&restored())));
    }

    #[test]
    fn refused_raw_mode_changes_nothing() {
        let out = review(&mut app(), vec![Step::Later(key('q'))], Fault::RawOn);
        assert_eq!(out.result.unwrap_err().to_string(), "raw mode refused");
        assert_eq!(out.frames, 0);
        Device::with(|d| {
            assert!(
                d.written.is_empty(),
                "nothing is written to a terminal never entered"
            );
            assert_eq!(d.script.len(), 1);
        });
    }

    /// Before 2026-10-31 the guard was armed only after the alternate screen was entered, so a
    /// failed write (stdout closed) returned with the terminal still in raw mode.
    #[test]
    fn a_failed_alternate_screen_entry_turns_raw_mode_back_off() {
        let out = review(&mut app(), vec![Step::Later(key('q'))], Fault::Write);
        assert_eq!(out.result.unwrap_err().to_string(), "stdout closed");
        assert_eq!(out.frames, 0);
        assert!(!Device::with(|d| d.raw), "raw mode left on");
    }
}
