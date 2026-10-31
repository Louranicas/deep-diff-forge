// DDF integration: the daemon serves requests in parallel under a byte budget (2026-10-30), closing the one-engine residual risk.
// Design/status: docs/OPERATIONS_AND_DAEMON.md#concurrency
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcrates--deep-diff-forge-daemon--src--admission
//! Request admission: how many request bytes the daemon works on at once.
//!
//! Requests run in parallel, one per connection thread, so their working memory adds up. Each
//! request is admitted with a weight (its line length, or more for a request whose work is larger
//! than its line), and the weights of the requests running together stay within
//! [`MAX_IN_FLIGHT_BYTES`]. A request heavier than the whole budget runs alone, so it is never
//! refused for its weight. Admission is first come, first served: a waiting heavy request is not
//! overtaken by lighter ones. No request waits forever either: after [`ADMISSION_WAIT`] it is
//! refused as [`Busy`], and its place in the queue is given up so the requests behind it move on.

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The request bytes the daemon works on at once. Two requests at the 64 MiB patch budget run
/// together; a third waits.
pub const MAX_IN_FLIGHT_BYTES: usize = 128 * 1024 * 1024;

/// How long a request waits for its turn before it is refused as [`Busy`]. A client's read
/// deadline is the same 30 s, so a longer wait would only answer a client that has gone.
pub const ADMISSION_WAIT: Duration = Duration::from_secs(30);

/// Tickets are served in the order they were taken.
#[derive(Debug, Default)]
struct Queue {
    /// The next ticket to hand out.
    next: u64,
    /// The ticket whose turn it is.
    serving: u64,
    /// Tickets whose request stopped waiting before its turn came; their turn is skipped.
    abandoned: BTreeSet<u64>,
    /// The summed weight of the requests running now.
    in_flight: usize,
    /// How many requests are running now.
    running: usize,
}

impl Queue {
    /// Move the turn past `serving` and past every abandoned ticket after it.
    fn advance(&mut self) {
        self.serving = self.serving.wrapping_add(1);
        while self.abandoned.remove(&self.serving) {
            self.serving = self.serving.wrapping_add(1);
        }
    }
}

/// A request waited [`ADMISSION_WAIT`] without its turn coming: the daemon is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    /// How long it waited.
    pub waited: Duration,
}

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the daemon is busy: no room in its in-flight budget after {} ms; retry later",
            self.waited.as_millis()
        )
    }
}

impl std::error::Error for Busy {}

/// A first-come, first-served gate on the request bytes in flight.
#[derive(Debug)]
pub struct Admission {
    budget: usize,
    wait: Duration,
    queue: Mutex<Queue>,
    turn: Condvar,
}

impl Default for Admission {
    fn default() -> Self {
        Self::new(MAX_IN_FLIGHT_BYTES)
    }
}

/// The admission clock door: how long a request may still wait.
#[expect(
    clippy::disallowed_methods,
    reason = "the admission clock door: bounds how long a request waits for its turn"
)]
fn now() -> Instant {
    Instant::now()
}

impl Admission {
    /// A gate that keeps the summed weight of running requests within `budget`, refusing a
    /// request that waited [`ADMISSION_WAIT`].
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self::with_wait(budget, ADMISSION_WAIT)
    }

    /// [`Admission::new`] with another longest wait.
    #[must_use]
    pub fn with_wait(budget: usize, wait: Duration) -> Self {
        Self {
            budget,
            wait,
            queue: Mutex::new(Queue::default()),
            turn: Condvar::new(),
        }
    }

    /// Wait for this request's turn and for room in the budget, then hold `weight` bytes of it
    /// until the returned permit is dropped. Nothing else can mint a [`Permit`].
    ///
    /// # Errors
    /// [`Busy`] when the turn has not come within the gate's longest wait. The request's place
    /// is given up, so the requests behind it are not held up by one that left.
    pub fn admit(&self, weight: usize) -> Result<Permit<'_>, Busy> {
        self.admit_for(weight, self.wait)
    }

    /// [`Admission::admit`] waiting at most `wait`.
    fn admit_for(&self, weight: usize, wait: Duration) -> Result<Permit<'_>, Busy> {
        let start = now();
        let deadline = start + wait;
        let mut queue = self.lock();
        let ticket = queue.next;
        queue.next = queue.next.wrapping_add(1);
        let mut blocked = false;
        while queue.serving != ticket || !self.fits(&queue, weight) {
            blocked = true;
            let remaining = deadline.saturating_duration_since(now());
            if remaining.is_zero() {
                queue.abandoned.insert(ticket);
                if queue.serving == ticket {
                    queue.abandoned.remove(&ticket);
                    queue.advance();
                }
                drop(queue);
                self.turn.notify_all();
                return Err(Busy { waited: wait });
            }
            queue = self
                .turn
                .wait_timeout(queue, remaining)
                .map_or_else(|poisoned| poisoned.into_inner().0, |(queue, _)| queue);
        }
        queue.advance();
        queue.in_flight = queue.in_flight.saturating_add(weight);
        queue.running += 1;
        let admitted = Admitted {
            waited: if blocked {
                now() - start
            } else {
                Duration::ZERO
            },
            in_flight: queue.in_flight,
            running: queue.running,
        };
        drop(queue);
        // The next ticket may fit beside this one.
        self.turn.notify_all();
        Ok(Permit {
            admission: self,
            weight,
            admitted,
        })
    }

    /// Whether a request of `weight` may start now: it fits the budget, or nothing is running.
    fn fits(&self, queue: &Queue, weight: usize) -> bool {
        queue.running == 0 || queue.in_flight.saturating_add(weight) <= self.budget
    }

    /// The queue holds only counters and a set of tickets, each updated by single statements,
    /// so it is consistent even after a panic elsewhere poisoned the lock.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The summed weight of the requests running now.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.lock().in_flight
    }

    /// How many requests are running now.
    #[must_use]
    pub fn running(&self) -> usize {
        self.lock().running
    }
}

/// Proof that a request was admitted; dropping it returns its weight to the budget, also when
/// the request panicked.
#[derive(Debug)]
#[must_use = "a request is admitted only while its permit is held"]
pub struct Permit<'a> {
    admission: &'a Admission,
    weight: usize,
    admitted: Admitted,
}

/// What admission looked like for one request: how long it waited for its turn (zero when it
/// did not have to), and the bytes in flight and requests running once it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub waited: Duration,
    pub in_flight: usize,
    pub running: usize,
}

impl Permit<'_> {
    /// How this request was admitted.
    #[must_use]
    pub const fn admitted(&self) -> Admitted {
        self.admitted
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut queue = self.admission.lock();
        queue.in_flight = queue.in_flight.saturating_sub(self.weight);
        queue.running = queue.running.saturating_sub(1);
        drop(queue);
        self.admission.turn.notify_all();
    }
}

/// The requests that were read and not yet answered, across every connection. A stop waits for
/// them ([`Answering::wait_idle`]), so it does not cut off an answer that is being built or
/// written. Unlike a [`Permit`], an [`Answer`] is held until the response is flushed.
#[derive(Debug, Default)]
pub struct Answering {
    count: Mutex<usize>,
    idle: Condvar,
}

impl Answering {
    /// Count a request as being answered until the returned guard is dropped.
    pub fn start(&self) -> Answer<'_> {
        *self.lock() += 1;
        Answer { answering: self }
    }

    /// How many requests are being answered now.
    #[must_use]
    pub fn count(&self) -> usize {
        *self.lock()
    }

    /// Wait at most `wait` for every request being answered to finish; whether they all did.
    #[must_use]
    pub fn wait_idle(&self, wait: Duration) -> bool {
        let count = self.lock();
        let (count, _) = self
            .idle
            .wait_timeout_while(count, wait, |count| *count > 0)
            .unwrap_or_else(PoisonError::into_inner);
        *count == 0
    }

    /// A counter, consistent after a panic elsewhere poisoned the lock.
    fn lock(&self) -> MutexGuard<'_, usize> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One request being answered; dropping it (on success, error or unwind) ends the count.
#[derive(Debug)]
pub struct Answer<'a> {
    answering: &'a Answering,
}

impl Drop for Answer<'_> {
    fn drop(&mut self) {
        let mut count = self.answering.lock();
        *count = count.saturating_sub(1);
        drop(count);
        self.answering.idle.notify_all();
    }
}

#[cfg(test)]
mod tests {
    /// A stop waits for the answers in progress and no longer; an answer ends its count when it
    /// is dropped, also by an unwind.
    #[test]
    fn answering_counts_until_each_answer_is_dropped() {
        let answering = Answering::default();
        assert!(answering.wait_idle(Duration::ZERO), "nothing to wait for");
        let first = answering.start();
        let second = answering.start();
        assert_eq!(answering.count(), 2);
        assert!(
            !answering.wait_idle(Duration::from_millis(20)),
            "two still answering"
        );
        drop(first);
        assert_eq!(answering.count(), 1);
        std::thread::scope(|scope| {
            // The waiter would wait 10 s; it must answer within 2 s of the last answer's drop, so
            // only a wakeup from that drop passes.
            let (woken, rx) = std::sync::mpsc::channel();
            let answering = &answering;
            scope.spawn(move || woken.send(answering.wait_idle(Duration::from_secs(10))));
            std::thread::park_timeout(Duration::from_millis(50));
            drop(second);
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(2)),
                Ok(true),
                "the waiter is woken by the last answer"
            );
        });
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _answer = answering.start();
            panic!("a request that panics while answering");
        }));
        assert!(unwound.is_err());
        assert_eq!(answering.count(), 0);
    }

    use super::*;
    use std::sync::mpsc;
    use std::thread;

    /// Long enough that a request which should start does; a request which must wait is checked
    /// with the same bound, so a pass is never a timing accident in the waiting direction.
    const SETTLE: Duration = Duration::from_millis(300);
    /// Every gate in these tests gives up after this, so a broken gate fails a test, never hangs it.
    const WAIT: Duration = Duration::from_secs(5);

    fn gate(budget: usize) -> Admission {
        Admission::with_wait(budget, WAIT)
    }

    #[test]
    fn the_budget_and_the_wait_are_as_documented() {
        assert_eq!(MAX_IN_FLIGHT_BYTES, 128 << 20);
        assert_eq!(ADMISSION_WAIT, Duration::from_secs(30));
        let default = Admission::default();
        assert_eq!((default.budget, default.wait), (128 << 20, ADMISSION_WAIT));
    }

    #[test]
    fn light_requests_run_together() {
        let gate = gate(100);
        let first = gate.admit(40).unwrap();
        let second = gate.admit(40).unwrap();
        // Neither waited; each saw the bytes and requests running once it started.
        assert_eq!(
            (first.admitted(), second.admitted()),
            (
                Admitted {
                    waited: Duration::ZERO,
                    in_flight: 40,
                    running: 1
                },
                Admitted {
                    waited: Duration::ZERO,
                    in_flight: 80,
                    running: 2
                },
            )
        );
        assert_eq!((gate.running(), gate.in_flight()), (2, 80));
        drop(first);
        assert_eq!((gate.running(), gate.in_flight()), (1, 40));
        drop(second);
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
    }

    #[test]
    fn a_request_exactly_filling_the_budget_starts() {
        let gate = gate(100);
        let _first = gate.admit(60).unwrap();
        let _second = gate.admit(40).unwrap();
        assert_eq!(gate.in_flight(), 100);
    }

    #[test]
    fn a_request_over_the_budget_runs_alone() {
        let gate = gate(100);
        let _alone = gate.admit(500).unwrap();
        assert_eq!((gate.running(), gate.in_flight()), (1, 500));
    }

    #[test]
    fn a_request_that_does_not_fit_waits_until_room_is_freed() {
        let gate = gate(100);
        let held = gate.admit(70).unwrap();
        let (started, rx) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(|| {
                let permit = gate.admit(40);
                started.send(permit.map(|p| p.admitted()).ok()).unwrap();
            });
            assert!(
                rx.recv_timeout(SETTLE).is_err(),
                "70 + 40 started over a 100 budget"
            );
            drop(held);
            let admitted = rx
                .recv_timeout(WAIT * 2)
                .unwrap()
                .expect("it never started");
            // It records a real wait: the request it waited for was held for SETTLE, minus the
            // moments before the waiter took its ticket.
            assert!(admitted.waited >= SETTLE / 2, "{admitted:?}");
            assert_eq!((admitted.in_flight, admitted.running), (40, 1));
        });
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
    }

    #[test]
    fn a_waiting_heavy_request_is_not_overtaken() {
        let gate = gate(100);
        let held = gate.admit(60).unwrap();
        let (order, rx) = mpsc::channel();
        thread::scope(|scope| {
            let heavy = order.clone();
            let gate = &gate;
            scope.spawn(move || {
                let permit = gate.admit(60);
                heavy.send(("heavy", permit.is_ok())).unwrap();
            });
            // The heavy request holds the first ticket before the light one asks.
            for _ in 0..1000 {
                if gate.lock().next >= 2 {
                    break;
                }
                thread::park_timeout(Duration::from_millis(1));
            }
            assert_eq!(gate.lock().next, 2, "the heavy request never asked");
            let light = order.clone();
            scope.spawn(move || {
                let permit = gate.admit(10);
                light.send(("light", permit.is_ok())).unwrap();
            });
            // 60 + 10 would fit, but the light request's turn comes after the heavy one's.
            assert!(
                rx.recv_timeout(SETTLE).is_err(),
                "a later request overtook a waiting one"
            );
            // Once room is freed both run (60 + 10 fits), so their messages may arrive in either
            // order; the order of admission was proven above.
            drop(held);
            let mut started = [
                rx.recv_timeout(WAIT * 2).unwrap(),
                rx.recv_timeout(WAIT * 2).unwrap(),
            ];
            started.sort_unstable();
            assert_eq!(started, [("heavy", true), ("light", true)]);
        });
    }

    #[test]
    fn a_request_that_waits_too_long_is_busy_and_gives_up_its_turn() {
        let gate = Admission::with_wait(100, Duration::from_millis(200));
        let _held = gate.admit(90).unwrap();
        // Its turn came but it never fitted: refused, and the turn moves on.
        let busy = gate.admit(50).unwrap_err();
        assert_eq!(busy.waited, Duration::from_millis(200));
        assert_eq!(
            busy.to_string(),
            "the daemon is busy: no room in its in-flight budget after 200 ms; retry later"
        );
        // So the next request that fits starts at once rather than waiting behind it.
        let _light = gate.admit(10).unwrap();
        assert_eq!((gate.running(), gate.in_flight()), (2, 100));
    }

    #[test]
    fn a_request_that_leaves_before_its_turn_does_not_stall_the_queue() {
        let gate = gate(100);
        let held = gate.admit(90).unwrap();
        thread::scope(|scope| {
            // The heavy request waits at the head of the queue (ticket 1).
            let heavy = scope.spawn(|| gate.admit(60).map(drop).is_ok());
            for _ in 0..1000 {
                if gate.lock().next >= 2 {
                    break;
                }
                thread::park_timeout(Duration::from_millis(1));
            }
            assert_eq!(gate.lock().next, 2, "the heavy request never asked");
            // Ticket 2 gives up behind it: marked abandoned, not served.
            assert!(gate.admit_for(5, Duration::from_millis(100)).is_err());
            assert!(gate.lock().abandoned.contains(&2));
            drop(held);
            assert!(heavy.join().unwrap(), "the heavy request never started");
        });
        // The turn skipped the abandoned ticket, so a new request starts at once.
        let after = gate.admit_for(10, Duration::from_millis(100));
        assert!(
            after.is_ok(),
            "the queue stalled behind a request that left"
        );
        drop(after);
        assert!(gate.lock().abandoned.is_empty());
        assert_eq!(gate.lock().serving, 4);
    }

    #[test]
    fn a_panicking_request_returns_its_weight() {
        let gate = gate(100);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = gate.admit(90).unwrap();
            panic!("the request failed");
        }));
        assert!(outcome.is_err());
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
        let _next = gate.admit(90).unwrap();
    }
}
