// DDF anchor A-01: every CLI entry point admits its arguments against a closed set before acting.
// Design/status: vault Strategy/Anchor Task List 2026-10-27.md (A-01).
//! Closed option sets. A misspelt or invented option is refused with exit 2 instead of being
//! ignored: before this, `--require-file` (one letter short) skipped the guard it named and a gate
//! passed every empty patch.

use deep_diff_forge_core::{display_safe, exit};

/// The arguments one command accepts.
pub(crate) struct Spec {
    /// Options that stand alone.
    pub flags: &'static [&'static str],
    /// Options that take the next argument as their value.
    pub valued: &'static [&'static str],
    /// The most positional arguments the command accepts.
    pub positionals: usize,
}

/// The valued options that may be given more than once; each is read with `flag_values`. Any
/// other valued option given twice is refused: before, the first value was used and a later one
/// was never read or validated, so `--max-bytes 4096 --max-bytes x` and
/// `--repo . --repo /nonexistent` both exited 0 (bug hunt 2026-11-01).
pub(crate) const REPEATABLE: [&str; 4] = ["--address", "--path", "--allow-repo", "--workload"];

/// Every valued option any [`Spec`] declares. [`help_requested`] reads a `-h` that follows one of
/// these as its value, not as a request for help; [`admit`] checks, in debug builds, that a spec
/// declares no valued option missing here, so the two lists cannot drift.
pub(crate) const VALUED: [&str; 20] = [
    "--layout",
    "--parallel",
    "--address",
    "--path",
    "--max-bytes",
    "--cursor",
    "--repo",
    "--socket",
    "--allow-repo",
    "--log-format",
    "--format",
    "--out",
    "--client",
    "--cmd",
    "--cols",
    "--rows",
    "--findings",
    "--workload",
    "--corpus",
    "--family",
];

/// Whether `args` ask for help: `--help` or `-h` standing as an option, before any `--` and not
/// as the value of the option before it. Words after `--` belong to someone else: `envelope --
/// --stdin-patch --help` wraps the child's help, and before this printed envelope's own help with
/// exit 0 and no `ddf.result.v1` object; `--hunks --path -h` selects the file named `-h`.
pub(crate) fn help_requested(args: &[String]) -> bool {
    let mut previous: Option<&str> = None;
    for arg in args.iter().map(String::as_str) {
        if arg == "--" {
            return false;
        }
        if matches!(arg, "--help" | "-h") && !previous.is_some_and(|p| VALUED.contains(&p)) {
            return true;
        }
        previous = Some(arg);
    }
    false
}

/// A command that takes no arguments at all.
pub(crate) const NONE: Spec = Spec {
    flags: &[],
    valued: &[],
    positionals: 0,
};

/// Check `args` against `spec`; the error names the first argument that does not fit. The
/// positionals come back in order, so a command never rescans `args` for them: before, the daemon
/// took the first non-option argument as its subcommand, which in `--socket PATH health` was PATH.
pub(crate) fn admit<'a>(args: &'a [String], spec: &Spec) -> Result<Vec<&'a str>, String> {
    debug_assert!(
        spec.valued.iter().all(|option| VALUED.contains(option)),
        "a valued option is missing from args::VALUED"
    );
    let mut positionals = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let arg = arg.as_str();
        if spec.valued.contains(&arg) {
            if rest.next().is_none_or(|value| value.starts_with("--")) {
                return Err(format!("{arg} needs a value"));
            }
            if seen.contains(&arg) && !REPEATABLE.contains(&arg) {
                return Err(format!("{arg} given more than once"));
            }
            seen.push(arg);
        } else if spec.flags.contains(&arg) {
        } else if arg.starts_with('-') {
            return Err(format!("unknown option: {arg}"));
        } else {
            if positionals.len() == spec.positionals {
                return Err(format!("unexpected argument: {arg}"));
            }
            positionals.push(arg);
        }
    }
    Ok(positionals)
}

/// [`admit`]'s positionals, or exit 2 with the reason and the command's usage on stderr and
/// nothing on stdout.
pub(crate) fn admit_or_exit<'a>(args: &'a [String], spec: &Spec, usage: &str) -> Vec<&'a str> {
    admit(args, spec).unwrap_or_else(|reason| {
        eprintln!("error: {}", display_safe(&reason));
        eprintln!("usage: {usage}");
        std::process::exit(exit::USAGE);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        flags: &["--json", "--require-files"],
        valued: &["--layout"],
        positionals: 1,
    };

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn declared_flags_values_and_positionals_are_admitted() {
        assert_eq!(admit(&args(&[]), &SPEC), Ok(vec![]));
        assert_eq!(
            admit(
                &args(&["x.rs", "--json", "--layout", "inline", "--require-files"]),
                &SPEC
            ),
            Ok(vec!["x.rs"])
        );
        // A flag given twice is harmless and stays admitted.
        assert_eq!(admit(&args(&["--json", "--json"]), &SPEC), Ok(vec![]));
    }

    /// A valued option's value is never a positional, wherever the option stands.
    #[test]
    fn a_valued_option_s_value_is_never_a_positional() {
        assert_eq!(
            admit(&args(&["--layout", "inline", "x.rs"]), &SPEC),
            Ok(vec!["x.rs"])
        );
        assert_eq!(admit(&args(&["--layout", "inline"]), &SPEC), Ok(vec![]));
    }

    /// A single-valued option given twice is refused whatever its values, so neither a bad later
    /// value nor an appended override is silently dropped; the repeatable ones stay admitted.
    #[test]
    fn a_single_valued_option_given_twice_is_refused() {
        for list in [
            &["--layout", "inline", "--layout", "bogus"][..],
            &["--layout", "inline", "x.rs", "--layout", "inline"],
        ] {
            assert_eq!(
                admit(&args(list), &SPEC),
                Err("--layout given more than once".to_owned()),
                "{list:?}"
            );
        }
        let repeatable = Spec {
            flags: &[],
            valued: &["--path", "--address", "--allow-repo", "--workload"],
            positionals: 0,
        };
        for option in REPEATABLE {
            assert_eq!(
                admit(&args(&[option, "a", option, "b"]), &repeatable),
                Ok(vec![]),
                "{option}"
            );
        }
    }

    #[test]
    fn help_is_an_option_before_any_separator_and_never_a_value() {
        for (list, asked) in [
            (&["--help"][..], true),
            (&["-h"], true),
            (&["7", "--help"], true),
            (&["--json", "-h"], true),
            (&["--hunks", "--path", "-h"], false),
            (&["--hunks", "--path", "-h", "-h"], true),
            (&["--", "--stdin-patch", "--help"], false),
            (&["--", "council", "-h"], false),
            (&["--json"], false),
            (&[], false),
        ] {
            assert_eq!(help_requested(&args(list)), asked, "{list:?}");
        }
    }

    #[test]
    fn misspelt_invented_or_reshaped_options_are_refused_by_name() {
        for (list, reason) in [
            (&["--require-file"][..], "unknown option: --require-file"),
            (&["--requre-files"], "unknown option: --requre-files"),
            (&["--jsno"], "unknown option: --jsno"),
            (&["--json=true"], "unknown option: --json=true"),
            (&["-j"], "unknown option: -j"),
            (&["--layout=inline"], "unknown option: --layout=inline"),
        ] {
            assert_eq!(
                admit(&args(list), &SPEC),
                Err(reason.to_owned()),
                "{list:?}"
            );
        }
    }

    #[test]
    fn a_valued_option_needs_its_value_and_positionals_are_bounded() {
        assert_eq!(
            admit(&args(&["--layout"]), &SPEC),
            Err("--layout needs a value".to_owned())
        );
        assert_eq!(
            admit(&args(&["--layout", "--json"]), &SPEC),
            Err("--layout needs a value".to_owned())
        );
        assert_eq!(
            admit(&args(&["a.rs", "b.rs"]), &SPEC),
            Err("unexpected argument: b.rs".to_owned())
        );
        assert_eq!(
            admit(&args(&["x"]), &NONE),
            Err("unexpected argument: x".to_owned())
        );
    }
}
