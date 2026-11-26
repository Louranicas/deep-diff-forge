//! Test only: what a scan costs, measured so that a test tells a slow algorithm from a slow
//! machine. A cost test runs the same work at sizes that double, on one thread and with one
//! clock, and bounds the ratio of the times at two sizes sixteen times apart ([`growth`]): the
//! machine's speed and the build's cancel out, so a linear scan passes on a slow or loaded
//! runner and a quadratic one fails on a fast one. An absolute bound stays only as a guard
//! against a hang (the tests' `within_deadline`), never as the measure of the cost.
//!
//! Why sixteen times and not two (lane Q3 of 0.11.1, 2026-11-27): measured from 1 KiB to 16 MiB,
//! one size at a time, the scans these tests hold are linear (fitted exponents 0.998 to 1.008 in
//! a debug build, 0.92 to 1.06 in a release build; logs `~/.cache/ddf-t/w111Q3/logs/m1-*`), yet
//! a single doubling measured 3.0 to 4.5 on GitHub's macos-26 runner, where the clock is wall
//! time, and up to 3.5 here with sixteen workers ranking at once: the larger size was descheduled
//! in every round (52 ms of CPU time in 104 ms of wall time). A bound on one doubling tolerates a
//! slowdown of only 1.5 times (3 over 2) at one of its two sizes, and a quadratic scan's doubling
//! (4) is too near a linear one's (2) for more. Sixteen times apart a linear scan grows 16 times
//! and a quadratic one 256: the bound is their geometric middle, 64, an exponent of 1.5
//! ([`EXPONENT_BOUND`]), which tolerates a slowdown of 4 times at either size and still fails a
//! quadratic scan with 4 times to spare.

use std::time::Duration;

/// The two sizes judged are `2^SPAN` times apart, or, where fewer than `SPAN` doublings lie
/// below the larger, the smallest size and the larger, at least `2^MIN_SPAN` apart: a scan that
/// is quadratic with a large constant already costs [`JUDGED_FROM`] at the second or third size
/// and fails there, before the ladder reaches a size that takes seconds (a plant measured 24 s at
/// the fifth size, 2026-11-27).
const SPAN: u32 = 4;
const MIN_SPAN: u32 = 2;

/// The bound on the exponent of a scan's cost between the two sizes of a pair (the cost grows
/// `span^exponent` times while the size grows `span` times): a linear scan's is 1, a quadratic
/// one's 2. Sixteen times apart the bound is a ratio of 64, the geometric middle of 16 and 256;
/// four and eight times apart, 8 and 22.6. Measured (2026-11-27, debug build, sixteen workers
/// ranking at once, 3,328 pairs sixteen times apart): ratios of at most 26 in the least of the
/// rounds and 35 in a single round.
pub(crate) const EXPONENT_BOUND: f64 = 1.5;

/// The sizes [`growth`] climbs: from `n` halved this many times, doubling up to `2n`. Ten
/// halvings start a scan that is quadratic with a large constant (a rescan of the line before
/// each token) at a size where it is still fast; six reached the hang guard first.
const LADDER: u32 = 10;

/// A pair of sizes below the last is judged only when its larger time is at least this (under
/// it a ratio is mostly fixed costs and noise); the last pair is always judged. This chooses
/// which pairs count; it bounds no cost.
const JUDGED_FROM: Duration = Duration::from_millis(50);

/// The smaller time of a pair is read as at least this. Linux's thread CPU time
/// (`/proc/thread-self/schedstat`) is read as of the scheduler's last tick, so a reading can be
/// short by up to a tick (1 ms at 1000 Hz, 4 ms at 250 Hz; readings measured 2026-11-27 came in
/// steps of about 1 ms), and a small or zero reading would make a ratio of noise.
const RESOLUTION: Duration = Duration::from_millis(4);

/// At most this many rounds of a judged pair: a round more of both sizes is run only while the
/// exponent is at or over [`EXPONENT_BOUND`], and the smaller time of each size is kept, so a
/// host that slows one sample (another process in the caches, a descheduled thread) does not
/// decide the answer.
const ROUNDS: usize = 3;

/// The time a scan spends: the thread's own CPU time where Linux reports it, so a busy host
/// does not fail the bound (a starved run measured 1.8 s of CPU in over 10 s of wall time),
/// and wall time elsewhere.
pub(crate) struct ScanClock {
    pub(crate) kind: &'static str,
    cpu_ns: Option<u64>,
    wall: std::time::Instant,
}

impl ScanClock {
    pub(crate) fn start() -> Self {
        let cpu_ns = thread_cpu_ns();
        #[expect(
            clippy::disallowed_methods,
            reason = "test door: the fallback clock where no CPU time is reported"
        )]
        let wall = std::time::Instant::now();
        let kind = if cpu_ns.is_some() {
            "thread CPU time"
        } else {
            "wall time"
        };
        Self { kind, cpu_ns, wall }
    }

    pub(crate) fn elapsed(&self) -> Duration {
        match (self.cpu_ns, thread_cpu_ns()) {
            (Some(then), Some(now)) => Duration::from_nanos(now - then),
            _ => self.wall.elapsed(),
        }
    }
}

/// The calling thread's CPU time in nanoseconds: the first field of Linux's
/// `/proc/thread-self/schedstat`, or `None` where that file does not exist.
fn thread_cpu_ns() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok()
}

/// The cost of one piece of work at two sizes [`SPAN`] doublings apart (fewer at the ladder's
/// foot): the smaller time at each size over the rounds run.
pub(crate) struct Growth {
    low: usize,
    at_low: Duration,
    high: usize,
    pub(crate) at_high: Duration,
    /// `high` is `low` doubled this many times.
    doublings: u32,
    kind: &'static str,
    rounds: usize,
}

/// Measure `work` on the calling thread at sizes that double from `n >> LADDER` up to `2n`
/// (`n` rounded down to a multiple of `2^LADDER`), each once, and return the first judged pair
/// (a size and the size [`SPAN`] doublings below it, or the smallest size) whose exponent stays
/// at or over [`EXPONENT_BOUND`] after [`ROUNDS`], or else the last pair. The small sizes cost a
/// quarter of the whole and make a quadratic scan fail at a size where it is still fast, rather
/// than at the hang guard. `work(size)` builds its input (not timed) and returns the scan,
/// which is timed.
pub(crate) fn growth<S: FnOnce()>(n: usize, mut work: impl FnMut(usize) -> S) -> Growth {
    let mut time = |size: usize| {
        let scan = work(size);
        let clock = ScanClock::start();
        scan();
        (clock.elapsed(), clock.kind)
    };
    let base = (n >> LADDER).max(1);
    let top = LADDER + 1;
    let mut times = Vec::with_capacity(top as usize + 1);
    for step in 0..=top {
        let (at_high, kind) = time(base << step);
        times.push(at_high);
        if step < MIN_SPAN {
            continue;
        }
        let low = step.saturating_sub(SPAN);
        let last = step == top;
        if !last && at_high < JUDGED_FROM {
            continue;
        }
        let mut cost = Growth {
            low: base << low,
            at_low: times[low as usize],
            high: base << step,
            at_high,
            doublings: step - low,
            kind,
            rounds: 1,
        };
        while cost.rounds < ROUNDS && cost.over() {
            cost.at_low = cost.at_low.min(time(cost.low).0);
            cost.at_high = cost.at_high.min(time(cost.high).0);
            cost.rounds += 1;
        }
        if last || cost.over() {
            return cost;
        }
        times[low as usize] = cost.at_low;
        times[step as usize] = cost.at_high;
    }
    unreachable!("the last pair returns")
}

impl Growth {
    /// How many times the cost grew from the smaller size to the larger, the smaller time read
    /// as at least [`RESOLUTION`].
    pub(crate) fn ratio(&self) -> f64 {
        self.at_high.as_secs_f64() / self.at_low.max(RESOLUTION).as_secs_f64()
    }

    /// How many times the size grew.
    fn span(&self) -> f64 {
        f64::from(1_u32 << self.doublings)
    }

    /// The exponent of the cost's growth: `ratio = span^exponent`.
    pub(crate) fn exponent(&self) -> f64 {
        self.ratio().ln() / self.span().ln()
    }

    /// Whether the cost grew by an exponent of [`EXPONENT_BOUND`] or more.
    pub(crate) fn over(&self) -> bool {
        self.exponent() >= EXPONENT_BOUND
    }

    /// The two sizes and their times, for a test's message.
    pub(crate) fn times(&self) -> String {
        format!(
            "{:?} at size {} and {:?} at size {} ({}, the least of {} rounds), ratio {:.1} \
             (exponent {:.2})",
            self.at_low,
            self.low,
            self.at_high,
            self.high,
            self.kind,
            self.rounds,
            self.ratio(),
            self.exponent()
        )
    }

    /// The cost bound of a scan: fails unless the cost grew by an exponent under
    /// [`EXPONENT_BOUND`] between the two sizes, and names both times (printed in any case).
    pub(crate) fn assert_linear(&self, what: &str) {
        println!("{what}: {}", self.times());
        assert!(
            !self.over(),
            "{what} is not linear: the cost grew {:.1} times when the size grew {} times, an \
             exponent of {:.2}, {EXPONENT_BOUND} or more: {}",
            self.ratio(),
            self.span(),
            self.exponent(),
            self.times()
        );
    }
}
