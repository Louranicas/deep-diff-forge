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
//! overtaken by lighter ones, so nothing waits forever.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// The request bytes the daemon works on at once. Two requests at the 64 MiB patch budget run
/// together; a third waits.
pub const MAX_IN_FLIGHT_BYTES: usize = 128 * 1024 * 1024;

/// Tickets are served in the order they were taken.
#[derive(Debug, Default)]
struct Queue {
    /// The next ticket to hand out.
    next: u64,
    /// The ticket whose turn it is.
    serving: u64,
    /// The summed weight of the requests running now.
    in_flight: usize,
    /// How many requests are running now.
    running: usize,
}

/// A first-come, first-served gate on the request bytes in flight.
#[derive(Debug)]
pub struct Admission {
    budget: usize,
    queue: Mutex<Queue>,
    turn: Condvar,
}

impl Default for Admission {
    fn default() -> Self {
        Self::new(MAX_IN_FLIGHT_BYTES)
    }
}

impl Admission {
    /// A gate that keeps the summed weight of running requests within `budget`.
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            queue: Mutex::new(Queue::default()),
            turn: Condvar::new(),
        }
    }

    /// Wait for this request's turn and for room in the budget, then hold `weight` bytes of it
    /// until the returned permit is dropped. Nothing else can mint a [`Permit`].
    pub fn admit(&self, weight: usize) -> Permit<'_> {
        let mut queue = self.lock();
        let ticket = queue.next;
        queue.next = queue.next.wrapping_add(1);
        while !(queue.serving == ticket && self.fits(&queue, weight)) {
            queue = self
                .turn
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
        queue.serving = queue.serving.wrapping_add(1);
        queue.in_flight = queue.in_flight.saturating_add(weight);
        queue.running += 1;
        drop(queue);
        // The next ticket may fit beside this one.
        self.turn.notify_all();
        Permit {
            admission: self,
            weight,
        }
    }

    /// Whether a request of `weight` may start now: it fits the budget, or nothing is running.
    fn fits(&self, queue: &Queue, weight: usize) -> bool {
        queue.running == 0 || queue.in_flight.saturating_add(weight) <= self.budget
    }

    /// The queue holds only counters, each updated by a single statement, so it is consistent
    /// even after a panic elsewhere poisoned the lock.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Long enough that a request which should start does; a request which must wait is checked
    /// with the same bound, so a pass is never a timing accident in the waiting direction.
    const SETTLE: Duration = Duration::from_millis(300);

    #[test]
    fn light_requests_run_together() {
        let gate = Admission::new(100);
        let first = gate.admit(40);
        let second = gate.admit(40);
        assert_eq!((gate.running(), gate.in_flight()), (2, 80));
        drop(first);
        assert_eq!((gate.running(), gate.in_flight()), (1, 40));
        drop(second);
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
    }

    #[test]
    fn a_request_exactly_filling_the_budget_starts() {
        let gate = Admission::new(100);
        let _first = gate.admit(60);
        let _second = gate.admit(40);
        assert_eq!(gate.in_flight(), 100);
    }

    #[test]
    fn a_request_over_the_budget_runs_alone() {
        let gate = Admission::new(100);
        let _alone = gate.admit(500);
        assert_eq!((gate.running(), gate.in_flight()), (1, 500));
    }

    #[test]
    fn a_request_that_does_not_fit_waits_until_room_is_freed() {
        let gate = Admission::new(100);
        let held = gate.admit(70);
        let (started, rx) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(|| {
                let _permit = gate.admit(40);
                started.send(()).unwrap();
            });
            assert!(
                rx.recv_timeout(SETTLE).is_err(),
                "70 + 40 started over a 100 budget"
            );
            drop(held);
            rx.recv_timeout(Duration::from_secs(10))
                .expect("the waiting request never started after room was freed");
        });
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
    }

    #[test]
    fn a_waiting_heavy_request_is_not_overtaken() {
        let gate = Admission::new(100);
        let held = gate.admit(60);
        let (order, rx) = mpsc::channel();
        thread::scope(|scope| {
            let heavy = order.clone();
            let gate = &gate;
            scope.spawn(move || {
                let _permit = gate.admit(60);
                heavy.send("heavy").unwrap();
            });
            // The heavy request holds the first ticket before the light one asks.
            while gate.lock().next < 2 {
                thread::yield_now();
            }
            let light = order.clone();
            scope.spawn(move || {
                let _permit = gate.admit(10);
                light.send("light").unwrap();
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
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
                rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            ];
            started.sort_unstable();
            assert_eq!(started, ["heavy", "light"]);
        });
    }

    #[test]
    fn a_panicking_request_returns_its_weight() {
        let gate = Admission::new(100);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = gate.admit(90);
            panic!("the request failed");
        }));
        assert!(outcome.is_err());
        assert_eq!((gate.running(), gate.in_flight()), (0, 0));
        let _next = gate.admit(90);
    }
}
