//! Test only: what a scan costs, measured so that a test tells a slow algorithm from a slow
//! machine. A cost test runs the same work at a size and at twice that size, on one thread and
//! with one clock, and bounds the ratio of the two times ([`doubling`]): the machine's speed and
//! the build's cancel out, so a linear scan passes on a slow or loaded runner and a quadratic one
//! fails on a fast one. An absolute bound stays only as a guard against a hang (the tests'
//! `within_deadline`), never as the measure of the cost.

use std::time::Duration;

/// A linear scan's cost about doubles when its input doubles, a quadratic one's about
/// quadruples. Bounded under 3: the tests' linear scans measured 1.8 to 2.4 on a desktop of 16
/// threads, idle, loaded or pinned to one CPU, and 2.7 in one sample beside fifteen other
/// threads ranking (2026-11-25); a scan reaches 3 only when its quadratic part is already as
/// large as its linear part at the smaller size.
pub(crate) const RATIO_BOUND: f64 = 3.0;

/// The sizes [`doubling`] climbs: from `n` halved this many times, doubling up to `2n`. Ten
/// halvings start a scan that is quadratic with a large constant (a rescan of the line before
/// each token) at a size where it is still fast; six reached the hang guard first.
const LADDER: u32 = 10;

/// A step of the ladder below the last is judged only when its larger time is at least this
/// (under it a ratio is mostly fixed costs and noise); the last step is always judged. This
/// chooses which steps count; it bounds no cost.
const JUDGED_FROM: Duration = Duration::from_millis(50);

/// At most this many rounds of a judged step: a round more is run only while its ratio is at or
/// over [`RATIO_BOUND`], and the smaller time of each size is kept, so a host that slows one
/// sample (another process in the caches) does not decide the answer.
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

/// The cost of one piece of work at a size and at twice that size: the smaller time at each
/// size over the rounds run.
pub(crate) struct Doubling {
    size: usize,
    at_size: Duration,
    pub(crate) at_twice: Duration,
    kind: &'static str,
    rounds: usize,
}

/// Measure `work` on the calling thread at sizes that double from `n >> LADDER` up to `2n`
/// (`n` rounded down to a multiple of `2^LADDER`), each once, and return the first judged step
/// whose ratio stays at or over [`RATIO_BOUND`] after [`ROUNDS`], or else the last step. The
/// small steps cost a quarter of the whole and make a quadratic scan fail at a size where it is
/// still fast, rather than at the hang guard. `work(size)` builds its input (not timed) and
/// returns the scan, which is timed.
pub(crate) fn doubling<S: FnOnce()>(n: usize, mut work: impl FnMut(usize) -> S) -> Doubling {
    let mut time = |size: usize| {
        let scan = work(size);
        let clock = ScanClock::start();
        scan();
        (clock.elapsed(), clock.kind)
    };
    let base = (n >> LADDER).max(1);
    let mut below = time(base).0;
    for step in 0..=LADDER {
        let size = base << step;
        let (at_twice, kind) = time(2 * size);
        let mut cost = Doubling {
            size,
            at_size: below,
            at_twice,
            kind,
            rounds: 1,
        };
        let last = step == LADDER;
        if last || at_twice >= JUDGED_FROM {
            while cost.rounds < ROUNDS && cost.ratio() >= RATIO_BOUND {
                cost.at_size = cost.at_size.min(time(size).0);
                cost.at_twice = cost.at_twice.min(time(2 * size).0);
                cost.rounds += 1;
            }
            if last || cost.ratio() >= RATIO_BOUND {
                return cost;
            }
        }
        below = cost.at_twice;
    }
    unreachable!("the last step returns")
}

impl Doubling {
    /// How many times the cost grew when the size doubled.
    pub(crate) fn ratio(&self) -> f64 {
        self.at_twice.as_secs_f64() / self.at_size.as_secs_f64()
    }

    /// The two sizes and their times, for a test's message.
    pub(crate) fn times(&self) -> String {
        format!(
            "{:?} at size {} and {:?} at size {} ({}, the least of {} rounds), ratio {:.2}",
            self.at_size,
            self.size,
            self.at_twice,
            2 * self.size,
            self.kind,
            self.rounds,
            self.ratio()
        )
    }

    /// The cost bound of a scan: fails unless doubling the size grew the time under
    /// [`RATIO_BOUND`] times, and names both times (printed in any case).
    pub(crate) fn assert_linear(&self, what: &str) {
        println!("{what}: {}", self.times());
        assert!(
            self.ratio() < RATIO_BOUND,
            "{what} is not linear: the cost grew {:.2} times, {RATIO_BOUND} or more, when the \
             size doubled: {}",
            self.ratio(),
            self.times()
        );
    }
}
