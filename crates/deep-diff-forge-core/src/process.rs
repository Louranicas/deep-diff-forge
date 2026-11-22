//! Bounded child processes. A program DDF waits for is waited for under a deadline and with
//! capped output, and is killed and reaped when it overruns, so a hung or chatty child cannot hang
//! or bloat its caller (production readiness 2026-11-04: `tuicr` and the council's Git calls
//! waited without either bound).
//!
//! `clippy.toml` disallows the unbounded waits (`Command::output`, `Command::status`,
//! `Child::wait`, `Child::wait_with_output`) outside a reviewed door, so a new caller reaches for
//! [`run_bounded`] rather than writing its own wait.
//!
//! Each child runs in a process group of its own, and the whole group is killed on every way out
//! (time limit, output cap, and after the child exits by itself), so nothing it started in its
//! group outlives the call (S3.17a: a judge that left a grandchild holding its output stretched an
//! agent's stop from 30 s to 75 s). A process that leaves the group (`setsid`, or a double fork
//! that makes its own group) is not killed by DDF: the call still returns by its deadline, but that
//! process runs on. Under Claude Code 2.1.330 the client ends a timed-out hook's descendants,
//! including those in their own group or session (measured 2026-11-22); a process that detaches
//! from them is ended by no one. Since the child is not in the caller's group, a signal from the
//! terminal (Ctrl-C) reaches the caller and not the child; a caller killed by it leaves the child
//! to finish on its own.

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
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
/// The child is made the leader of a new process group (the caller's own setting of a group is
/// replaced), and the group is killed when the call ends, however it ends. The readers and the
/// writer are joined only while the deadline lasts and detached after it: a process that left the
/// group can hold the pipes open, and joining would wait for it (bug hunt 2026-10-28, H1-03, and
/// S3.17a). So the call returns by [`Limits::time`] plus about one poll and the time to kill and
/// reap; a child that exited in time whose pipes are still held at the deadline is
/// [`Stopped::TimedOut`].
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
        .process_group(0)
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
            // Before the reap, while the group's id is still the child's own.
            kill_group(&child);
            let _ = child.kill();
            let _ = child.wait();
            drop((writer, stdout_reader, stderr_reader));
            return Err(stop);
        }
        std::thread::sleep(POLL);
    };
    // The leader is reaped; what it left in its group goes now, so its pipes close.
    kill_group(&child);
    while !(stdout_reader.is_finished()
        && stderr_reader.is_finished()
        && writer.as_ref().is_none_or(JoinHandle::is_finished))
    {
        if deadline.checked_duration_since(Instant::now()).is_none() {
            // Detached: a process outside the group still holds a pipe.
            return Err(Stopped::TimedOut { limit: limits.time });
        }
        std::thread::sleep(POLL);
    }
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

/// Kill the process group `child` leads. Errors are ignored: an empty group is already gone.
fn kill_group(child: &Child) {
    if let Ok(group) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
    }
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

    /// What a test may add to a bound for a loaded machine: the call itself returns within about
    /// one poll of its deadline, so this is margin for the scheduler, not for the code.
    const MARGIN: Duration = Duration::from_secs(2);

    /// A `sleep` argument no other process has: `whole` seconds, this process's id and `n` in
    /// the fraction. It names a grandchild without a file to write its pid to.
    fn mark(whole: u32, n: u32) -> String {
        format!("{whole}.{:07}{n}", std::process::id())
    }

    /// Shell words that start `sleep <mark>` in the background, its output sent to `to` (empty:
    /// the child's own pipes), and wait until it runs `sleep` (before its exec the child is still
    /// the shell), so it exists when the next step ends the call.
    fn background_sleep(mark: &str, to: &str) -> String {
        format!(
            "sleep {mark} {to} & P=$!; \
             until [ \"$(cat /proc/$P/comm 2>/dev/null)\" = sleep ]; do sleep 0.01; done; "
        )
    }

    /// The running processes (zombies are not) whose command line is `sleep <mark>`.
    #[cfg(target_os = "linux")]
    fn sleepers(mark: &str) -> Vec<String> {
        let want = format!("sleep\0{mark}\0").into_bytes();
        let running = |pid: &String| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let state = stat.rsplit(')').next().unwrap_or_default().trim_start();
            !state.is_empty() && !state.starts_with('Z')
        };
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|pid| pid.bytes().all(|b| b.is_ascii_digit()))
            .filter(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| c == want))
            .filter(running)
            .collect()
    }

    /// The sleepers of `mark` still running after [`MARGIN`] for a kill to land; they are then
    /// killed, so no test leaves one behind.
    #[cfg(target_os = "linux")]
    fn left_running(mark: &str) -> Vec<String> {
        let started = Instant::now();
        loop {
            let left = sleepers(mark);
            if left.is_empty() || started.elapsed() > MARGIN {
                for pid in &left {
                    let _ = Command::new("kill").args(["-KILL", pid]).status();
                }
                return left;
            }
            std::thread::sleep(POLL);
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_leader_that_exits_while_a_grandchild_holds_stdout_returns_by_the_deadline() {
        let limits = Limits {
            time: Duration::from_millis(300),
            ..ROOMY
        };
        // In the child's group: killed when the leader exits, so the output is complete at once.
        let held = mark(31, 0);
        let started = Instant::now();
        let script = format!("{}printf done; exit 0", background_sleep(&held, ""));
        let done = run_bounded(&mut sh(&script), &[], limits).unwrap();
        assert_eq!(done.stdout, b"done");
        assert!(
            started.elapsed() < limits.time + MARGIN,
            "{:?}",
            started.elapsed()
        );
        assert_eq!(left_running(&held), Vec::<String>::new());
        // Out of the group (its own session): not killed, so the output never ends; the call
        // returns at its deadline all the same. The leader exits when the escaped process, out
        // of the group by then, signals it.
        let escaped = mark(32, 1);
        let script = format!(
            "L=$$; trap 'exit 0' USR1; setsid sh -c \"kill -USR1 $L; exec sleep {escaped}\" & \
             while :; do sleep 0.01; done"
        );
        let started = Instant::now();
        let late = run_bounded(&mut sh(&script), &[], limits);
        let took = started.elapsed();
        let left = left_running(&escaped);
        assert!(
            took < limits.time + MARGIN,
            "a process that held stdout after its parent exited held the caller for {took:?}"
        );
        assert!(matches!(late, Err(Stopped::TimedOut { .. })), "{late:?}");
        assert_eq!(left.len(), 1, "the escaped process did not hold the pipe");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_whole_group_is_killed_on_every_exit_path() {
        let capped = Limits {
            time: Duration::from_secs(5),
            stdout_bytes: 5,
            ..ROOMY
        };
        let timed = Limits {
            time: Duration::from_secs(1),
            ..ROOMY
        };
        // The grandchild's output goes nowhere, so only the group kill can end it.
        for (n, (path, tail, limits)) in (2..).zip([
            ("exit", "exit 0", ROOMY),
            ("timeout", "exec sleep 30", timed),
            ("cap", "exec yes", capped),
        ]) {
            let grandchild = mark(33, n);
            let script = format!("{}{tail}", background_sleep(&grandchild, ">/dev/null 2>&1"));
            let _ = run_bounded(&mut sh(&script), &[], limits);
            let left = left_running(&grandchild);
            assert!(
                left.is_empty(),
                "{path}: the grandchild {left:?} outlived the call"
            );
        }
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
