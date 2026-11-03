// DDF integration: a request served on the daemon socket stops working once its client has given up (expert circle 2026-11-03, C9).
// Design/status: docs/OPERATIONS_AND_DAEMON.md#concurrency
//! The cooperative execution deadline of the request this thread is serving.
//!
//! A socket client stops reading 30 s after it sent its request (`READ_TIMEOUT`). Before, the
//! daemon kept working on the request after that, holding a worker and its admission weight for
//! an answer nobody would read. The connection handler now runs each request [`within`] that
//! deadline, and the handler calls [`check`] between its stages (after a parse, an acquisition or
//! a ranking, and per line of a findings file). Once the deadline has passed, the request ends
//! with `RESOURCE_EXHAUSTED`, and its permit is released at once. A stage already running
//! finishes first: each is bounded by its own input budget, so the overrun is bounded too.
//! Outside [`within`] (the stdio `rpc` and MCP servers, tests), [`check`] never fails.

use crate::protocol::{RESOURCE_EXHAUSTED, RpcError};
use std::cell::Cell;
use std::time::Instant;

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Restores the deadline that was in force before [`within`], even on a panic.
struct Restore(Option<Instant>);

impl Drop for Restore {
    fn drop(&mut self) {
        DEADLINE.with(|deadline| deadline.set(self.0));
    }
}

/// Run `work` with `deadline` as this thread's execution deadline.
pub(crate) fn within<T>(deadline: Instant, work: impl FnOnce() -> T) -> T {
    let _restore = Restore(DEADLINE.with(|current| current.replace(Some(deadline))));
    work()
}

/// `Err(RESOURCE_EXHAUSTED)` once this thread's deadline has passed; `Ok` with none in force.
pub(crate) fn check() -> Result<(), RpcError> {
    match DEADLINE.with(Cell::get) {
        Some(deadline) if crate::metrics::now() >= deadline => Err(RpcError::new(
            RESOURCE_EXHAUSTED,
            "the request ran past its client's 30 s read deadline and was stopped; \
             send a smaller request or retry when the daemon is less busy",
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn check_fails_only_inside_a_passed_deadline_and_the_old_one_comes_back() {
        assert!(check().is_ok(), "no deadline in force");
        let now = crate::metrics::now();
        within(now + Duration::from_secs(60), || {
            assert!(check().is_ok(), "a deadline still ahead");
            within(now, || {
                let err = check().unwrap_err();
                assert_eq!(err.code, RESOURCE_EXHAUSTED);
                assert!(err.message.contains("read deadline"), "{}", err.message);
            });
            assert!(check().is_ok(), "the outer deadline is back");
        });
        assert!(check().is_ok());
        let unwound = std::panic::catch_unwind(|| within(now, || panic!("work")));
        assert!(unwound.is_err());
        assert!(check().is_ok(), "restored after a panic");
    }
}
