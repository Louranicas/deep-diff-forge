//! The signal vocabulary as data: every label a ranking can carry, with what it means for a
//! reader that is not a person.
//!
//! [`facts`] and [`remedy`] are exhaustive matches over [`RiskSignal`] and [`EvidenceSignal`], so a
//! new signal does not compile until it has its facts and its remedy. The labels themselves stay
//! where they are (`RiskSignal::label`, `EvidenceSignal::label`): tools outside this repository
//! read them there. Weights come from the functions the scorer uses (`RiskSignal::weight`), and
//! the sides from the rule table ([`signal_sides`]), so this document cannot disagree with a
//! ranking. For the path and shape signals a test holds the same: each is scored on probes
//! and must be found from exactly what its facts say it is read from.

use crate::evidence::{EvidenceSignal, Side, signal_sides};
use crate::risk::RiskSignal;

/// What a signal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalClass {
    /// The change could run code, reach outside, leak a credential or mislead a reader.
    Security,
    /// The change weakens a check that guards the code: a test, a lint, an error path.
    Integrity,
    /// A fact about the path or the size of the change, used to order the review.
    Shape,
}

impl SignalClass {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::Integrity => "integrity",
            Self::Shape => "shape",
        }
    }
}

/// How sure a signal is of what it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// The construct the signal names is on the line it points at.
    Direct,
    /// A guess from the shape of a change; never proof.
    Heuristic,
}

impl SignalKind {
    /// Stable label for machine output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Heuristic => "heuristic",
        }
    }
}

/// The basis of a signal read from the path, the status, the modes or the size of a change.
pub const PATH_BASIS: &str = "path";

/// The path and shape signals, in the order a ranking lists them.
pub const PATH_SIGNALS: [RiskSignal; 13] = [
    RiskSignal::PublicApiSurface,
    RiskSignal::NewFile,
    RiskSignal::DeletedFile,
    RiskSignal::BinaryChange,
    RiskSignal::DiffSuppressed,
    RiskSignal::LargeChange,
    RiskSignal::ManyHunks,
    RiskSignal::ConfigOrLockfile,
    RiskSignal::TestOnly,
    RiskSignal::GeneratedOrVendored,
    RiskSignal::HazardousText,
    RiskSignal::SymlinkChange,
    RiskSignal::ExecutableBit,
];

/// What is true of one signal in this version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalFacts {
    pub signal: RiskSignal,
    pub class: SignalClass,
    pub kind: SignalKind,
    /// The release that first wrote the label.
    pub since: &'static str,
    /// One line: what the signal says about a change.
    pub summary: &'static str,
}

impl SignalFacts {
    /// The label a ranking carries.
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.signal.label()
    }

    /// What the signal adds to a file's score, once per file.
    #[must_use]
    pub const fn weight(&self) -> u32 {
        self.signal.weight()
    }

    /// What the signal is read from: the path, status, modes or size of a change
    /// ([`PATH_BASIS`]), the text of its lines (for a code-evidence signal, every `basis` an
    /// item of it can carry, from the rules that set it: `crate::evidence::signal_bases`), or
    /// both.
    ///
    /// `hazardous_text` is both: the scorer looks at the path and at every changed line. It
    /// was listed as read from the path alone (independent review of the hooks, HR-L-6). A
    /// code-evidence signal whose items can stand on `hunk` was listed as `lexical` alone
    /// (integrated review of 0.9.0, VI1-L-2 and CI3).
    #[must_use]
    pub fn basis(&self) -> &'static [&'static str] {
        match self.signal {
            RiskSignal::Code(signal) => crate::evidence::signal_bases(signal),
            RiskSignal::HazardousText => &[PATH_BASIS, crate::evidence::EVIDENCE_BASIS],
            _ => &[PATH_BASIS],
        }
    }

    /// The sides of a hunk the signal is looked for on; empty for a signal that reads no line.
    /// `hazardous_text` is looked for on both, though a ranking points at no line for it.
    #[must_use]
    pub fn sides(&self) -> Vec<Side> {
        match self.signal {
            RiskSignal::Code(signal) => signal_sides(signal),
            RiskSignal::HazardousText => vec![Side::New, Side::Old],
            _ => Vec::new(),
        }
    }
}

/// The facts of `signal`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one arm per signal, kept in one place so none is missed"
)]
pub const fn facts(signal: RiskSignal) -> SignalFacts {
    use EvidenceSignal as E;
    use RiskSignal as R;
    use SignalClass::{Integrity, Security, Shape};
    let (class, since, summary) = match signal {
        R::PublicApiSurface => (
            Shape,
            "0.1.0",
            "The path looks like a public API surface (lib.rs, mod.rs, an api/ directory).",
        ),
        R::NewFile => (Shape, "0.1.0", "The file is added by the change."),
        R::DeletedFile => (Shape, "0.1.0", "The file is deleted by the change."),
        R::BinaryChange => (
            Shape,
            "0.1.0",
            "The change is binary: there is no text to review.",
        ),
        R::DiffSuppressed => (
            Security,
            "0.10.0",
            "Git was told not to show this text file's changes (an attribute such as -diff, or a NUL byte); DDF read them as text.",
        ),
        R::LargeChange => (
            Shape,
            "0.1.0",
            "Many lines are added or removed in the file.",
        ),
        R::ManyHunks => (Shape, "0.1.0", "The file is changed in many places."),
        R::ConfigOrLockfile => (Shape, "0.1.0", "The file is configuration or a lockfile."),
        R::TestOnly => (
            Shape,
            "0.1.0",
            "The path is a test path; the file earns no source bonus.",
        ),
        R::GeneratedOrVendored => (
            Shape,
            "0.1.0",
            "The path looks generated or vendored; the file earns no path or size bonus.",
        ),
        R::HazardousText => (
            Security,
            "0.3.0",
            "The path or a changed line holds bidi, invisible or control characters, or the path traverses or names .git.",
        ),
        R::SymlinkChange => (
            Security,
            "0.3.0",
            "A symbolic link is created, changed, removed or made from a file.",
        ),
        R::ExecutableBit => (Security, "0.3.0", "A file becomes executable."),
        R::Code(E::UnsafeAdded) => (Security, "0.5.0", "An added line uses Rust `unsafe`."),
        R::Code(E::ProcessSpawn) => (Security, "0.5.0", "An added line starts a process."),
        R::Code(E::NetworkFetch) => (
            Security,
            "0.5.0",
            "An added line fetches from or opens a connection to the network.",
        ),
        R::Code(E::DynamicEval) => (
            Security,
            "0.5.0",
            "An added line evaluates code built at run time.",
        ),
        R::Code(E::CiPrivilege) => (
            Security,
            "0.5.0",
            "A CI workflow gains a privileged trigger, wide permissions or inherited secrets.",
        ),
        R::Code(E::BuildHook) => (
            Security,
            "0.5.0",
            "A build script or an install hook is added or changed.",
        ),
        R::Code(E::SecretLike) => (
            Security,
            "0.5.0",
            "An added line holds something shaped like a credential.",
        ),
        R::Code(E::CheckSuppressed) => (
            Integrity,
            "0.5.0",
            "An added line silences a lint, a type check or a linter rule.",
        ),
        R::Code(E::TestRemoved) => (Integrity, "0.5.0", "A removed line was a test."),
        R::Code(E::TestSkipped) => (
            Integrity,
            "0.9.0",
            "An added line switches a test off where it stands: it still reads as a test and checks nothing.",
        ),
        R::Code(E::TrivialAssert) => (Integrity, "0.9.0", "An added assertion cannot fail."),
        R::Code(E::StubAdded) => (
            Integrity,
            "0.9.0",
            "An added line is a placeholder where code should be.",
        ),
        R::Code(E::ErrorSwallowed) => (
            Integrity,
            "0.9.0",
            "An added handler catches an error and drops it.",
        ),
        R::Code(E::PanicPathAdded) => (
            Integrity,
            "0.9.0",
            "An added line calls something that panics on failure (Rust `unwrap` or `expect`).",
        ),
    };
    SignalFacts {
        signal,
        class,
        kind: SignalKind::Direct,
        since,
        summary,
    }
}

/// Every signal of this version: the path and shape signals, then the code-evidence signals, in
/// the order a ranking lists them.
#[must_use]
pub fn vocabulary() -> Vec<SignalFacts> {
    PATH_SIGNALS
        .into_iter()
        .chain(EvidenceSignal::ALL.into_iter().map(RiskSignal::Code))
        .map(facts)
        .collect()
}

/// The signal with this label, if any.
#[must_use]
pub fn signal_named(label: &str) -> Option<RiskSignal> {
    vocabulary()
        .into_iter()
        .map(|f| f.signal)
        .find(|s| s.label() == label)
}

/// A substitute that does not clear a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAccepted {
    /// What an author might write to make the signal go away.
    pub substitute: &'static str,
    /// The label that substitute raises, or `None` when nothing in this version sees it (a
    /// declared miss: the substitute is still wrong).
    pub raises: Option<&'static str>,
}

/// A rule of a compiler or linter that refuses the construct before a review sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeLint {
    pub language: &'static str,
    pub tool: &'static str,
    pub rule: &'static str,
}

/// Why a signal matters and what to do about it, for the author of the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Remedy {
    /// Why a reviewer wants to see this.
    pub why: &'static str,
    /// Changes that answer the signal: either it no longer fires, or a reviewer can accept it.
    pub accept: &'static [&'static str],
    /// Changes that hide the signal without answering it.
    pub not_accepted: &'static [NotAccepted],
    /// Rules that stop the construct at compile or lint time, where one exists.
    pub native_lints: &'static [NativeLint],
}

/// A signal that orders the review and asks for nothing: there is nothing to repair.
const fn orders_the_review(why: &'static str) -> Remedy {
    Remedy {
        why,
        accept: &["Nothing to repair: the signal orders the review."],
        not_accepted: &[],
        native_lints: &[],
    }
}

/// The remedy of `signal`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one arm per signal, kept in one place so none is missed"
)]
pub const fn remedy(signal: RiskSignal) -> Remedy {
    use EvidenceSignal as E;
    use RiskSignal as R;
    match signal {
        R::PublicApiSurface => orders_the_review(
            "A change to a public surface can break every caller, so it is read first.",
        ),
        R::NewFile => orders_the_review(
            "A new file has no earlier version to compare with; all of it is new.",
        ),
        R::DeletedFile => Remedy {
            why: "A deleted file takes its behaviour and its tests with it.",
            accept: &["Say in the change why the file is no longer needed and what replaces it."],
            not_accepted: &[NotAccepted {
                substitute: "Emptying the file instead of deleting it, to avoid the signal.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::BinaryChange => Remedy {
            why: "A binary change cannot be read in a review.",
            accept: &["Say where the binary comes from and how to rebuild or verify it."],
            not_accepted: &[],
            native_lints: &[],
        },
        R::DiffSuppressed => Remedy {
            why: "An attribute or a byte that makes Git show a text file as binary hides its changes from every reviewer who reads `git diff`.",
            accept: &[
                "Remove the `-diff`, `binary` or `diff=` attribute that covers the source file, wherever it is set.",
                "Remove the NUL byte from the file, or say why a text file must hold one.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Storing the same code compressed or encoded, so that it is a real binary.",
                raises: Some("binary_change"),
            }],
            native_lints: &[],
        },
        R::LargeChange => Remedy {
            why: "A large change is harder to review and easier to approve without reading.",
            accept: &["Split the change so that each part has one purpose."],
            not_accepted: &[NotAccepted {
                substitute: "Moving lines into other files only to stay under the size limit.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::ManyHunks => Remedy {
            why: "Many separate edits in one file are easy to skim past.",
            accept: &["Split unrelated edits into separate changes."],
            not_accepted: &[],
            native_lints: &[],
        },
        R::ConfigOrLockfile => orders_the_review(
            "Configuration and lockfiles decide what is built and what every other check means.",
        ),
        R::TestOnly => orders_the_review(
            "A test path is read after source; a test that lost its force is reported by its own signal.",
        ),
        R::GeneratedOrVendored => Remedy {
            why: "A generated or vendored path is seldom read, so it must not hide hand-written code.",
            accept: &["Keep hand-written code out of generated and vendored directories."],
            not_accepted: &[NotAccepted {
                substitute: "Placing new source under a generated or vendored path to lower its rank.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::HazardousText => Remedy {
            why: "Bidi controls, invisible characters and traversing paths make a change read differently from what is applied.",
            accept: &[
                "Remove the hidden or control characters.",
                "Use a path inside the repository that does not traverse or name .git.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Building the same character at run time from an escape sequence.",
                raises: None,
            }],
            native_lints: &[NativeLint {
                language: "Rust",
                tool: "rustc",
                rule: "text_direction_codepoint_in_literal",
            }],
        },
        R::SymlinkChange => Remedy {
            why: "A link can point anywhere, and a later write through it lands there.",
            accept: &[
                "Replace the link with the file it stands for, or say why a link is needed and where it points.",
            ],
            not_accepted: &[],
            native_lints: &[],
        },
        R::ExecutableBit => Remedy {
            why: "An executable file can be run by a build, a hook or a person.",
            accept: &[
                "Keep the bit only on files that are meant to be run, and say what runs them.",
            ],
            not_accepted: &[],
            native_lints: &[],
        },
        R::Code(E::UnsafeAdded) => Remedy {
            why: "Unsafe code is outside the compiler's checks; a reviewer has to check its safety argument.",
            accept: &[
                "Use a safe construct.",
                "Keep the unsafe block and state its safety argument next to it for the reviewer.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Hiding the block inside a macro, or moving it into a generated or vendored path.",
                raises: None,
            }],
            native_lints: &[NativeLint {
                language: "Rust",
                tool: "rustc",
                rule: "unsafe_code",
            }],
        },
        R::Code(E::ProcessSpawn) => Remedy {
            why: "Starting a process runs whatever is at that path with the caller's rights.",
            accept: &[
                "Do the work in the program itself.",
                "Keep the call and say what is run, with which arguments, and under what limits.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Building the command as a string and evaluating it.",
                raises: Some("dynamic_eval"),
            }],
            native_lints: &[NativeLint {
                language: "Rust",
                tool: "clippy",
                rule: "disallowed_methods (configured for std::process::Command::new)",
            }],
        },
        R::Code(E::NetworkFetch) => Remedy {
            why: "A new connection moves data in or out, and can bring code in.",
            accept: &[
                "Say what is contacted and why, and pin what is fetched by hash where code or data is downloaded.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Calling curl or wget through a process instead.",
                raises: Some("process_spawn"),
            }],
            native_lints: &[],
        },
        R::Code(E::DynamicEval) => Remedy {
            why: "Code built at run time is not the code that was reviewed.",
            accept: &["Call the function directly, or parse the input as data."],
            not_accepted: &[NotAccepted {
                substitute: "Assembling the call from pieces so that no single line shows it.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::Code(E::CiPrivilege) => Remedy {
            why: "A privileged trigger, wide permissions or inherited secrets hand untrusted code the keys to the repository.",
            accept: &[
                "Use the least permission the job needs, and keep untrusted code out of privileged triggers.",
            ],
            not_accepted: &[],
            native_lints: &[],
        },
        R::Code(E::BuildHook) => Remedy {
            why: "A build script or an install hook runs on every machine that builds or installs.",
            accept: &["Say what the hook does and why it cannot be done at run time or by a tool."],
            not_accepted: &[],
            native_lints: &[],
        },
        R::Code(E::SecretLike) => Remedy {
            why: "A credential committed once is compromised for good, even after a revert.",
            accept: &[
                "Remove the value and revoke the credential if it is real.",
                "Read it at run time from the environment or a secret store.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Splitting or encoding the value so that it no longer looks like a credential.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::Code(E::CheckSuppressed) => Remedy {
            why: "A silenced check hides the next real defect it would have reported.",
            accept: &[
                "Fix what the check reports.",
                "Keep the suppression as narrow as possible and state the reason next to it.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Turning the check off in the tool's configuration file instead.",
                raises: None,
            }],
            native_lints: &[NativeLint {
                language: "Rust",
                tool: "clippy",
                rule: "allow_attributes_without_reason",
            }],
        },
        R::Code(E::TestRemoved) => Remedy {
            why: "A removed test is a guarantee that someone relied on and no longer has.",
            accept: &[
                "Keep the test and make it pass.",
                "Replace it with a test that checks the same behaviour, in the same change.",
            ],
            not_accepted: &[
                NotAccepted {
                    substitute: "Keeping the test but skipping it.",
                    raises: Some("test_skipped"),
                },
                NotAccepted {
                    substitute: "Keeping the test but emptying its body.",
                    raises: None,
                },
            ],
            native_lints: &[],
        },
        R::Code(E::TestSkipped) => Remedy {
            why: "A skipped test still reads as a test in the suite and checks nothing.",
            accept: &[
                "Make the test pass and remove the skip.",
                "If the test cannot run here, say why next to the skip and where it runs instead.",
            ],
            not_accepted: &[
                NotAccepted {
                    substitute: "Deleting the test instead of skipping it.",
                    raises: Some("test_removed"),
                },
                NotAccepted {
                    substitute: "Keeping the test running but asserting `true` (`assert!(true)`, `assert_eq!(true, true)`); other spellings of an assertion that cannot fail are not seen yet.",
                    raises: Some("trivial_assert"),
                },
            ],
            native_lints: &[],
        },
        R::Code(E::TrivialAssert) => Remedy {
            why: "An assertion that cannot fail passes whatever the code does.",
            accept: &["Assert on the behaviour the test is about."],
            not_accepted: &[NotAccepted {
                substitute: "Comparing a value with itself, or a constant with a constant, in another spelling.",
                raises: None,
            }],
            native_lints: &[NativeLint {
                language: "Rust",
                tool: "clippy",
                rule: "assertions_on_constants",
            }],
        },
        R::Code(E::StubAdded) => Remedy {
            why: "A placeholder compiles, and fails or does nothing only when it is reached.",
            accept: &[
                "Write the code.",
                "If the code is meant to come later, say so in the change and keep the placeholder out of paths that run.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Returning a default value where the placeholder was.",
                raises: None,
            }],
            native_lints: &[
                NativeLint {
                    language: "Rust",
                    tool: "clippy",
                    rule: "todo",
                },
                NativeLint {
                    language: "Rust",
                    tool: "clippy",
                    rule: "unimplemented",
                },
            ],
        },
        R::Code(E::ErrorSwallowed) => Remedy {
            why: "An error that is caught and dropped is a failure nobody hears of.",
            accept: &[
                "Handle the error, or pass it to the caller.",
                "If it is safe to ignore, say why next to the handler.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Logging the error and going on as if it had not happened.",
                raises: None,
            }],
            native_lints: &[],
        },
        R::Code(E::PanicPathAdded) => Remedy {
            why: "A call that panics on failure stops the program where the error could have been handled.",
            accept: &[
                "Handle the error, or return it to the caller.",
                "If failure cannot happen, say why in the message of `expect`.",
            ],
            not_accepted: &[NotAccepted {
                substitute: "Replacing the call with another that panics in the same case, such as an index.",
                raises: None,
            }],
            native_lints: &[
                NativeLint {
                    language: "Rust",
                    tool: "clippy",
                    rule: "unwrap_used",
                },
                NativeLint {
                    language: "Rust",
                    tool: "clippy",
                    rule: "expect_used",
                },
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_every_label_once_in_ranking_order() {
        let labels: Vec<&str> = vocabulary().iter().map(SignalFacts::label).collect();
        assert_eq!(
            labels,
            [
                "public_api_surface",
                "new_file",
                "deleted_file",
                "binary_change",
                "diff_suppressed",
                "large_change",
                "many_hunks",
                "config_or_lockfile",
                "test_only",
                "generated_or_vendored",
                "hazardous_text",
                "symlink_change",
                "executable_bit",
                "unsafe_added",
                "process_spawn",
                "network_fetch",
                "dynamic_eval",
                "ci_privilege",
                "build_hook",
                "secret_like",
                "check_suppressed",
                "test_removed",
                "test_skipped",
                "trivial_assert",
                "stub_added",
                "error_swallowed",
                "panic_path_added",
            ]
        );
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len());
    }

    #[test]
    fn a_label_finds_its_signal_and_an_unknown_one_finds_none() {
        assert_eq!(signal_named("new_file"), Some(RiskSignal::NewFile));
        assert_eq!(
            signal_named("test_removed"),
            Some(RiskSignal::Code(EvidenceSignal::TestRemoved))
        );
        assert_eq!(signal_named("New_File"), None);
        assert_eq!(signal_named(""), None);
    }

    #[test]
    fn weights_basis_and_sides_come_from_the_scorer_and_the_rule_table() {
        let by = |label: &str| {
            vocabulary()
                .into_iter()
                .find(|f| f.label() == label)
                .unwrap()
        };
        assert_eq!(by("public_api_surface").weight(), 5);
        assert_eq!(by("test_only").weight(), 0);
        assert_eq!(by("secret_like").weight(), 5);
        assert_eq!(by("check_suppressed").weight(), 2);
        assert_eq!(by("new_file").basis(), ["path"]);
        assert_eq!(by("unsafe_added").basis(), ["lexical"]);
        // a handler and its body, or a Rust construct over several lines (VI1-L-2, CI3)
        assert_eq!(by("panic_path_added").basis(), ["lexical", "hunk"]);
        assert_eq!(by("error_swallowed").basis(), ["lexical", "hunk"]);
        assert_eq!(by("dynamic_eval").basis(), ["lexical"]);
        assert_eq!(by("new_file").sides(), []);
        assert_eq!(by("hazardous_text").basis(), ["path", "lexical"]);
        assert_eq!(by("hazardous_text").sides(), [Side::New, Side::Old]);
        assert_eq!(by("unsafe_added").sides(), [Side::New]);
        assert_eq!(by("test_removed").sides(), [Side::Old]);
        // every code-evidence signal is looked for on some side
        for signal in EvidenceSignal::ALL {
            assert!(!signal_sides(signal).is_empty(), "{}", signal.label());
        }
    }

    #[test]
    fn classes_and_kinds_have_stable_labels() {
        assert_eq!(
            [
                SignalClass::Security.label(),
                SignalClass::Integrity.label(),
                SignalClass::Shape.label()
            ],
            ["security", "integrity", "shape"]
        );
        assert_eq!(
            [SignalKind::Direct.label(), SignalKind::Heuristic.label()],
            ["direct", "heuristic"]
        );
        let class = |label: &str| {
            vocabulary()
                .into_iter()
                .find(|f| f.label() == label)
                .unwrap()
                .class
        };
        assert_eq!(class("large_change"), SignalClass::Shape);
        assert_eq!(class("hazardous_text"), SignalClass::Security);
        assert_eq!(class("diff_suppressed"), SignalClass::Security);
        assert_eq!(class("secret_like"), SignalClass::Security);
        assert_eq!(class("check_suppressed"), SignalClass::Integrity);
        assert_eq!(class("test_removed"), SignalClass::Integrity);
        assert!(vocabulary().iter().all(|f| f.kind == SignalKind::Direct));
    }

    /// A remedy is text an agent acts on: none may be empty, and a substitute may only name a
    /// label that exists.
    #[test]
    fn every_remedy_is_complete_and_names_only_known_labels() {
        for facts in vocabulary() {
            let remedy = remedy(facts.signal);
            let label = facts.label();
            assert!(!remedy.why.trim().is_empty(), "{label}: why");
            assert!(!remedy.accept.is_empty(), "{label}: accept");
            assert!(
                remedy.accept.iter().all(|a| !a.trim().is_empty()),
                "{label}: an accept line is empty"
            );
            for not in remedy.not_accepted {
                assert!(!not.substitute.trim().is_empty(), "{label}: substitute");
                if let Some(raised) = not.raises {
                    assert!(signal_named(raised).is_some(), "{label}: raises {raised}");
                    assert_ne!(raised, label, "{label}: a substitute raises itself");
                }
            }
            for lint in remedy.native_lints {
                assert!(
                    ![lint.language, lint.tool, lint.rule]
                        .iter()
                        .any(|s| s.trim().is_empty()),
                    "{label}: lint"
                );
            }
            assert!(!facts.summary.trim().is_empty(), "{label}: summary");
            assert!(
                // 0.9.0: the five integrity labels, merged with the hooks in that release
                // 0.10.0: `diff_suppressed` (S3.20)
                ["0.1.0", "0.3.0", "0.5.0", "0.9.0", "0.10.0"].contains(&facts.since),
                "{label}: since {}",
                facts.since
            );
        }
    }

    /// Integrity and security signals ask for a repair; each says what does not count as one, or
    /// names a rule that prevents it. A reader must never be told only "this is bad".
    #[test]
    fn a_signal_that_asks_for_a_repair_says_what_is_not_one_or_how_to_prevent_it() {
        for signal in EvidenceSignal::ALL {
            let remedy = remedy(RiskSignal::Code(signal));
            assert!(
                !remedy.accept.is_empty(),
                "{}: no accepted repair",
                signal.label()
            );
        }
        for signal in [
            EvidenceSignal::CheckSuppressed,
            EvidenceSignal::TestRemoved,
            EvidenceSignal::SecretLike,
            EvidenceSignal::UnsafeAdded,
        ] {
            assert!(
                !remedy(RiskSignal::Code(signal)).not_accepted.is_empty(),
                "{}",
                signal.label()
            );
        }
    }

    /// Independent review of the hooks, HR-L-6: the facts of `hazardous_text` said it was read
    /// from the path and looked for on no side, while the scorer reads every added and removed
    /// line. No test compared the facts of a path signal with the scorer, so they could drift.
    ///
    /// Here every path and shape signal is scored on probes that differ from a plain change in
    /// one thing only: a hazard in an added line, in a removed line, in a context line, or in
    /// the path. A signal must be raised by a line of a side exactly when its facts list that
    /// side, by some line exactly when they list the line basis, and never by a context line.
    #[test]
    fn the_facts_of_every_path_signal_say_what_the_scorer_reads() {
        use deep_diff_forge_patch::parse;
        let signals = |patch: &str| {
            let file = parse(patch).unwrap().into_iter().next().unwrap();
            crate::risk::score_file(&file).1
        };
        let change = |path: &str, context: &str, removed: &str, added: &str| {
            format!(
                "--- a/{path}\n+++ b/{path}\n@@ -1,2 +1,2 @@\n {context}\n-{removed}\n+{added}\n"
            )
        };
        // a bidi control, an invisible character and a terminal escape: one hazard of each kind
        for hazard in ["\u{202e}", "\u{200b}", "\u{1b}[2J"] {
            let plain = signals(&change("src/a.rs", "x", "a", "b"));
            let in_added = signals(&change("src/a.rs", "x", "a", &format!("b{hazard}")));
            let in_removed = signals(&change("src/a.rs", "x", &format!("a{hazard}"), "b"));
            let in_context = signals(&change("src/a.rs", &format!("x{hazard}"), "a", "b"));
            for signal in PATH_SIGNALS {
                let facts = facts(signal);
                let label = facts.label();
                let raised_by =
                    |probe: &[RiskSignal]| probe.contains(&signal) && !plain.contains(&signal);
                assert_eq!(
                    raised_by(&in_added),
                    facts.sides().contains(&Side::New),
                    "{label}: an added line"
                );
                assert_eq!(
                    raised_by(&in_removed),
                    facts.sides().contains(&Side::Old),
                    "{label}: a removed line"
                );
                assert!(
                    !raised_by(&in_context),
                    "{label}: a context line is no change"
                );
                assert_eq!(
                    raised_by(&in_added) || raised_by(&in_removed),
                    facts.basis().contains(&crate::evidence::EVIDENCE_BASIS),
                    "{label}: the line basis"
                );
                // every one of them reads the path, the status, the modes or the size as well
                assert!(facts.basis().contains(&PATH_BASIS), "{label}");
            }
        }
        // the path alone raises it too, which is the other half of its basis
        let plain = signals(&change("src/a.rs", "x", "a", "b"));
        let in_path = signals(&change("src/a\u{202e}.rs", "x", "a", "b"));
        assert!(
            in_path.contains(&RiskSignal::HazardousText)
                && !plain.contains(&RiskSignal::HazardousText)
        );
        // and a code-evidence signal reads lines only, on one line or over several
        for signal in EvidenceSignal::ALL {
            let facts = facts(RiskSignal::Code(signal));
            assert!(
                !facts.basis().is_empty()
                    && facts
                        .basis()
                        .iter()
                        .all(|basis| ["lexical", "hunk"].contains(basis)),
                "{}: {:?}",
                facts.label(),
                facts.basis()
            );
            assert_eq!(facts.sides(), signal_sides(signal), "{}", facts.label());
        }
    }
}
