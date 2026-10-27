// DDF anchor A-02: one hardening set for every Git invocation. Design/status: vault
// Strategy/Anchor Task List 2026-10-27.md (A-02).
//! The hardening every DDF Git invocation applies, as data (`core` runs no processes).
//!
//! DDF spawns Git from two crates (`patch` for acquisition, `review` for council storage), and
//! their hardening had drifted apart. Measured on 2026-10-27: acquisition in a partial clone ran
//! `git fetch origin` (a lazy promisor fetch, a network effect from a read-only command), and
//! council storage honoured an inherited `GIT_DIR`, so `rev-parse HEAD` named another repository
//! while `--show-toplevel` still passed the top-level check. Both launchers now apply this set.

/// Global options: no optional locks, no replacement refs, no fsmonitor or hooks (which run
/// code), no transports at all (so no lazy fetch from a promisor remote), no external diff.
pub const ARGS: &[&str] = &[
    "--no-optional-locks",
    "--no-replace-objects",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "protocol.allow=never",
    "-c",
    "diff.external=",
];

/// Variables that would point Git at another repository, index or object store, or change how it
/// diffs; they are removed from the child's environment.
pub const ENV_REMOVED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
];

/// Variables set in the child: replacement objects, grafts and lazy fetches stay off even where
/// a repository's configuration would turn them on.
pub const ENV_SET: &[(&str, &str)] = &[
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_GRAFT_FILE", "/dev/null"),
    ("GIT_NO_LAZY_FETCH", "1"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_turns_off_transports_hooks_and_foreign_locations() {
        for pair in [
            ["-c", "protocol.allow=never"],
            ["-c", "core.hooksPath=/dev/null"],
            ["-c", "core.fsmonitor=false"],
        ] {
            assert!(ARGS.windows(2).any(|w| w == pair), "{pair:?}");
        }
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
        ] {
            assert!(ENV_REMOVED.contains(&name), "{name}");
        }
        assert!(ENV_SET.contains(&("GIT_NO_LAZY_FETCH", "1")));
    }
}
