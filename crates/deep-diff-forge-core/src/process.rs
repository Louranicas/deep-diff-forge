//! Bounded child processes. A program DDF waits for is waited for under a deadline and with
//! capped output, and is killed and reaped when it overruns, so a hung or chatty child cannot hang
//! or bloat its caller (production readiness 2026-11-04: `tuicr` and the council's Git calls
//! waited without either bound).
//!
//! `clippy.toml` disallows the unbounded waits (`Command::output`, `Command::status`,
//! `Child::wait`, `Child::wait_with_output`) outside a reviewed door, so a new caller reaches for
//! [`run_bounded`] rather than writing its own wait.

use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long between checks on a running child.
const POLL: Duration = Duration::from_millis(5);

/// The bounds a child runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Wall-clock time from spawn until the child must have exited.
    pub time: Duration,
    /// Largest standard output accepted; one byte more stops the child.
    pub stdout_bytes: u64,
    /// Standard error kept; the rest is read and dropped, so it can never stall the child.
    pub stderr_bytes: u64,
}

/// A child that exited within its [`Limits`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    /// The first [`Limits::stderr_bytes`] of standard error.
    pub stderr: Vec<u8>,
}

/// Why a child produced no [`Finished`].
#[derive(Debug)]
pub enum Stopped {
    /// It could not be started.
    Spawn(std::io::Error),
    /// It ran past [`Limits::time`]; it was killed and reaped.
    TimedOut { limit: Duration },
    /// It wrote more than [`Limits::stdout_bytes`]; it was killed and reaped.
    OutputLimit { limit_bytes: u64 },
    /// Waiting for it or reading its output failed.
    Io(std::io::Error),
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start the command: {e}"),
            Self::TimedOut { limit } => {
                write!(
                    f,
                    "the command ran past its {} s time limit",
                    limit.as_secs()
                )
            }
            Self::OutputLimit { limit_bytes } => {
                write!(f, "the command wrote more than {limit_bytes} bytes")
            }
            Self::Io(e) => write!(f, "reading the command failed: {e}"),
        }
    }
}

impl std::error::Error for Stopped {}

/// Run `command` with `input` on its standard input (none when empty) and its output piped,
/// under `limits`. Its pipes are set here; the caller sets the program, arguments, directory and
/// environment.
///
/// The readers are detached, not joined, when the child is stopped: a process the child started
/// can outlive the kill and hold its pipes open, and joining would wait for it, so the deadline
/// would not bound the caller (the same rule as Git acquisition, bug hunt 2026-10-28, H1-03).
///
/// # Errors
///
/// [`Stopped`], saying which bound was crossed or what failed.
#[expect(
    clippy::disallowed_methods,
    reason = "the bounded-process door: reads the clock, sleeps between polls and reaps a stopped child"
)]
pub fn run_bounded(
    command: &mut Command,
    input: &[u8],
    limits: Limits,
) -> Result<Finished, Stopped> {
    let deadline = Instant::now() + limits.time;
    let mut child = command
        .stdin(if input.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Stopped::Spawn)?;
    // Each pipe on its own thread, so a large input, output and error stream cannot block each
    // other. Dropping the writer's handle closes the child's input.
    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let over = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&over);
    let stdout = child.stdout.take();
    let stdout_reader = std::thread::spawn(move || {
        let mut kept = Vec::new();
        let read = stdout.map(|pipe| {
            pipe.take(limits.stdout_bytes.saturating_add(1))
                .read_to_end(&mut kept)
        });
        if kept.len() as u64 > limits.stdout_bytes {
            flag.store(true, Ordering::SeqCst);
        }
        (kept, read)
    });
    let stderr = child.stderr.take();
    let stderr_reader = std::thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(mut pipe) = stderr {
            let _ = pipe
                .by_ref()
                .take(limits.stderr_bytes)
                .read_to_end(&mut kept);
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        }
        kept
    });
    let status = loop {
        let stop = if over.load(Ordering::SeqCst) {
            Some(Stopped::OutputLimit {
                limit_bytes: limits.stdout_bytes,
            })
        } else if let Some(status) = child.try_wait().map_err(Stopped::Io)? {
            break status;
        } else if deadline.checked_duration_since(Instant::now()).is_some() {
            // Time left (no comparison operator, so a mutated bound cannot hide here).
            None
        } else {
            Some(Stopped::TimedOut { limit: limits.time })
        };
        if let Some(stop) = stop {
            let _ = child.kill();
            let _ = child.wait();
            drop((writer, stdout_reader, stderr_reader));
            return Err(stop);
        }
        std::thread::sleep(POLL);
    };
    let (stdout, read) = stdout_reader
        .join()
        .map_err(|_| Stopped::Io(std::io::Error::other("the output reader panicked")))?;
    let stderr = stderr_reader.join().unwrap_or_default();
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    // The child can exit before the reader sees the byte over the cap.
    if stdout.len() as u64 > limits.stdout_bytes {
        return Err(Stopped::OutputLimit {
            limit_bytes: limits.stdout_bytes,
        });
    }
    if let Some(Err(e)) = read {
        return Err(Stopped::Io(e));
    }
    Ok(Finished {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "test: spawns shells and times the bounds"
)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    const ROOMY: Limits = Limits {
        time: Duration::from_secs(20),
        stdout_bytes: 1 << 20,
        stderr_bytes: 64,
    };

    #[test]
    fn a_child_within_its_limits_returns_its_status_and_output() {
        let done = run_bounded(&mut sh("printf out; printf err >&2; exit 3"), &[], ROOMY).unwrap();
        assert_eq!(done.status.code(), Some(3));
        assert_eq!(done.stdout, b"out");
        assert_eq!(done.stderr, b"err");
    }

    #[test]
    fn input_reaches_the_child_and_large_streams_do_not_block_each_other() {
        // 4 MiB through `cat` both ways, with 1 MiB of stderr beside it: more than any pipe
        // buffer, so a reader or writer on the waiting thread would deadlock.
        let input = vec![b'x'; 4 << 20];
        let limits = Limits {
            stdout_bytes: 8 << 20,
            ..ROOMY
        };
        let done = run_bounded(
            &mut sh("head -c 1048576 /dev/zero >&2; cat"),
            &input,
            limits,
        )
        .unwrap();
        assert!(done.status.success());
        assert_eq!(done.stdout, input);
        assert_eq!(done.stderr.len(), 64, "stderr is kept only up to its cap");
    }

    #[test]
    fn output_up_to_the_cap_is_kept_and_one_byte_more_stops_the_child() {
        let limits = Limits {
            stdout_bytes: 5,
            ..ROOMY
        };
        let done = run_bounded(&mut sh("printf 12345"), &[], limits).unwrap();
        assert_eq!(done.stdout, b"12345");
        let over = run_bounded(&mut sh("printf 123456"), &[], limits).unwrap_err();
        assert!(
            matches!(over, Stopped::OutputLimit { limit_bytes: 5 }),
            "{over}"
        );
        // A child that keeps writing is stopped, not waited for.
        let started = Instant::now();
        let endless = run_bounded(&mut sh("yes"), &[], limits).unwrap_err();
        assert!(matches!(endless, Stopped::OutputLimit { .. }), "{endless}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_child_past_its_deadline_is_stopped_even_when_a_grandchild_holds_its_pipes() {
        let limits = Limits {
            time: Duration::from_millis(300),
            ..ROOMY
        };
        let started = Instant::now();
        // The grandchild keeps stdout open after its parent is killed; the caller must not wait
        // for it.
        let late = run_bounded(&mut sh("sleep 30 & exec sleep 30"), &[], limits).unwrap_err();
        let took = started.elapsed();
        assert!(
            matches!(late, Stopped::TimedOut { limit } if limit == limits.time),
            "{late}"
        );
        assert!(took >= limits.time, "stopped before its deadline: {took:?}");
        assert!(
            took < Duration::from_secs(10),
            "the deadline did not bound the caller: {took:?}"
        );
    }

    #[test]
    fn a_program_that_cannot_start_is_a_spawn_failure() {
        let missing = run_bounded(
            &mut Command::new("/nonexistent/ddf-no-such-program"),
            &[],
            ROOMY,
        )
        .unwrap_err();
        assert!(matches!(missing, Stopped::Spawn(_)), "{missing}");
        assert!(
            missing
                .to_string()
                .starts_with("could not start the command")
        );
        assert_eq!(
            Stopped::TimedOut {
                limit: Duration::from_secs(120)
            }
            .to_string(),
            "the command ran past its 120 s time limit"
        );
        assert_eq!(
            Stopped::OutputLimit { limit_bytes: 9 }.to_string(),
            "the command wrote more than 9 bytes"
        );
    }
}
