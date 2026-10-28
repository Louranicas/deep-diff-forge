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
    let mut positionals = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let arg = arg.as_str();
        if spec.valued.contains(&arg) {
            if rest.next().is_none_or(|value| value.starts_with("--")) {
                return Err(format!("{arg} needs a value"));
            }
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
