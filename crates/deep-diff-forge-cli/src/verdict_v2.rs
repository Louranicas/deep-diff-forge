// DDF integration: the agent loop's stop hook (train 1) reads the judge's verdict document here.
// Design/status: docs/reviews/vesper-v4-round3-report-20261114.md (VD4R3-M-1)
//! A `forge-warden.verdict.v2` document, checked member by member.
//!
//! The stop hook lets an agent stop on a pass, so a document is a verdict only when it is a
//! whole one. This follows the schema forge-warden ships (`schemas/verdict.v2.schema.json` at
//! its 0.2.0): every member the schema requires, of its type, with its pattern or its closed set
//! of values, in the nested objects as well; the lists the schema says are never empty; the
//! `exit` triple that belongs to the `verdict`; and a `summary` that counts the `rules` beside
//! it. Two earlier checks were shallower and each let a document through that said two things
//! at once (review V4, VD4R2-M-2 and VD4R3-M-1).
//!
//! Open on purpose, so that a later forge-warden under the same schema name is still read:
//! a member this version does not know, a rule `kind` and a note `code` that are new (they must
//! still be lowercase names). Everything else the schema fixes is fixed here, the members it
//! makes optional included: when one is present it has the schema's form. The table of what
//! the schema names is in the test `every_member_the_schema_names_is_read`.
//!
//! A `forge-warden.verdict.v3` document (forge-warden's main branch, for 0.3.0) is read the
//! same way against its own schema (`schemas/verdict.v3.schema.json`), which differs from v2
//! in three places, each read for what it means (S3.21):
//! - `seals.claims` may also be bound (ADR 0012): `base`, `head` and `merge_base`, full
//!   object ids of one length (40 or 64 digits), with `verified: true`. forge-warden says it
//!   made the patch itself from a copy of the repository it checked. The stop hook gives no
//!   `--repo`, cannot check that and does not use it: the claims never change an outcome,
//!   and the verdict is bound to the hook's patch by the seals alone. The unverified form is
//!   v2's, with no `merge_base`.
//! - a rule of kind `builtin` (M7-Q64) has the id `builtin:` and a policy id, and only such a
//!   rule has; it holds (severity and outcome `hold`) and no waiver covers its violations.
//! - a note `code` may be `claims_unverified`, which the open note codes already admit.
//!
//! A document that names any other schema is no verdict of either.

use serde_json::Value;

/// The schema name every verdict carries.
pub(crate) const SCHEMA: &str = "forge-warden.verdict.v2";

/// The schema name of a version 3 verdict.
pub(crate) const SCHEMA_V3: &str = "forge-warden.verdict.v3";

/// The verdict schemas this build reads, as a sentence names them.
pub(crate) const READS: &str = "forge-warden.verdict.v2 and forge-warden.verdict.v3";

/// A version of the verdict schema this build reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Version {
    V2,
    V3,
}

impl Version {
    /// The version `document` names, when it is one this build reads.
    pub(crate) fn named_by(document: &Value) -> Option<Self> {
        match document["schema"].as_str()? {
            SCHEMA => Some(Self::V2),
            SCHEMA_V3 => Some(Self::V3),
            _ => None,
        }
    }

    /// The schema name of this version.
    pub(crate) const fn schema(self) -> &'static str {
        match self {
            Self::V2 => SCHEMA,
            Self::V3 => SCHEMA_V3,
        }
    }
}

/// `verdict`, and the `exit` triple that belongs to it.
const OUTCOMES: [(&str, u64, &str, &str); 3] = [
    ("pass", 0, "pass", "proceed"),
    ("fail", 7, "policy_fail", "change_input"),
    ("hold", 8, "policy_hold", "review_evidence"),
];

/// Whether `value` is 64 lowercase hex digits: a SHA-256 as forge-warden writes one.
pub(crate) fn is_sha256(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn is_sha256_or_null(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || is_sha256(value))
}

/// `YYYY-MM-DD`, digits only: the schema's pattern, not a calendar.
fn is_date(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        let bytes = text.as_bytes();
        bytes.len() == 10
            && bytes.iter().enumerate().all(|(at, b)| {
                if at == 4 || at == 7 {
                    *b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            })
    })
}

/// A lowercase name of letters, digits and `_`, starting with a letter: the form of every rule
/// kind and note code.
fn is_name(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.starts_with(|c: char| c.is_ascii_lowercase())
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// A rule id: lowercase words of letters and digits joined by single hyphens.
fn is_rule_id(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.starts_with(|c: char| c.is_ascii_lowercase())
            && text.split('-').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
    })
}

fn is_one_of(value: &Value, allowed: &[&str]) -> bool {
    value.as_str().is_some_and(|text| allowed.contains(&text))
}

/// The members of `value` that are not strings although the schema says they are, if present.
fn optional_strings(value: &Value, members: &[&str]) -> bool {
    members
        .iter()
        .all(|member| value.get(*member).is_none_or(Value::is_string))
}

/// v3's bound claims: `base`, `head` and `merge_base` of one length, 40 or 64 lowercase hex
/// digits, and `verified` is `true`.
fn is_bound(claims: &Value) -> bool {
    let length = claims["base"].as_str().map(str::len);
    claims["verified"] == true
        && matches!(length, Some(40 | 64))
        && ["base", "head", "merge_base"].iter().all(|member| {
            is_commit(&claims[*member]) && claims[*member].as_str().map(str::len) == length
        })
}

fn seals_defect(seals: &Value, version: Version) -> Option<&'static str> {
    if !seals.is_object() {
        return Some("`seals` is not an object");
    }
    if !(is_sha256(&seals["policy_sha256"])
        && is_sha256(&seals["rank_sha256"])
        && is_sha256(&seals["patch_sha256"]))
    {
        return Some("a seal in `seals` is not a SHA-256");
    }
    if !is_sha256_or_null(seals.get("patch_facts_sha256")) {
        return Some("`seals.patch_facts_sha256` is missing or not a SHA-256 or null");
    }
    if !is_one_of(&seals["provenance"], &["produced", "supplied"]) {
        return Some("`seals.provenance` is not `produced` or `supplied`");
    }
    let ranked_by = &seals["ranked_by"];
    if !(ranked_by["version"].is_string() && is_sha256_or_null(ranked_by.get("sha256"))) {
        return Some("`seals.ranked_by` lacks its version or its digest");
    }
    match seals.get("git") {
        Some(Value::Null) => {}
        Some(git) if git.is_object() => {
            let approved = git["admission"] == "approved";
            let admitted = if approved {
                is_sha256(&git["allowlist_sha256"])
            } else {
                git["admission"] == "unpinned" && git.get("allowlist_sha256") == Some(&Value::Null)
            };
            if !(git["path"].is_string()
                && git["version"].is_string()
                && is_sha256(&git["sha256"])
                && admitted)
            {
                return Some("`seals.git` is not a whole Git admission");
            }
        }
        _ => return Some("`seals.git` is missing"),
    }
    if let Some(claims) = seals.get("claims") {
        let commit = |member: &str| claims.get(member).is_none_or(is_commit);
        let asserted =
            claims.is_object() && claims["verified"] == false && commit("base") && commit("head");
        match version {
            Version::V2 if !asserted => {
                return Some("`seals.claims` is not an unverified claim of two commits");
            }
            Version::V3
                if !((asserted && claims.get("merge_base").is_none()) || is_bound(claims)) =>
            {
                return Some(
                    "`seals.claims` is neither an unverified claim of two commits nor a bound one of three",
                );
            }
            _ => {}
        }
    }
    None
}

/// A commit as a claim names one: 4 to 64 lowercase hex digits.
fn is_commit(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        (4..=64).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Whether `violation` is whole: its message, and each member the schema names, when present,
/// in the schema's form. `evidence` is a list of citations (a hunk address, a side, a line
/// from 1); `evidence_truncated` is present only as `true`; `waived_by` is the index of a
/// waiver. A malformed one of these beside a waived rule was a pass (review V4 round 4,
/// VD4R4-M-1).
fn violation_is_whole(violation: &Value) -> bool {
    let citations = violation.get("evidence").is_none_or(|evidence| {
        evidence.as_array().is_some_and(|citations| {
            citations.iter().all(|citation| {
                is_sha256(&citation["address"])
                    && is_one_of(&citation["side"], &["new", "old"])
                    && citation["line"].as_u64().is_some_and(|line| line >= 1)
            })
        })
    });
    violation["message"].is_string()
        && optional_strings(violation, &["path", "signal"])
        && citations
        && violation
            .get("evidence_truncated")
            .is_none_or(|truncated| truncated == true)
        && violation.get("waived_by").is_none_or(Value::is_u64)
}

fn rule_defect(rule: &Value, version: Version) -> Option<&'static str> {
    // v3: the id of a built-in rule is `builtin:` and a policy id; no other rule's is
    let builtin = version == Version::V3 && rule["kind"] == "builtin";
    let id = if builtin {
        rule["id"]
            .as_str()
            .and_then(|id| id.strip_prefix("builtin:"))
            .is_some_and(|id| is_rule_id(&Value::from(id)))
    } else {
        is_rule_id(&rule["id"])
    };
    if !(id
        && is_name(&rule["kind"])
        && is_one_of(&rule["severity"], &["fail", "hold"])
        && is_one_of(&rule["outcome"], &["pass", "waived", "hold", "fail"])
        && rule["why"].is_string())
    {
        return Some(
            "a rule lacks its id, kind, severity, outcome or reason, or has one malformed",
        );
    }
    let whole = rule["violations"]
        .as_array()
        .is_some_and(|violations| violations.iter().all(violation_is_whole));
    if !whole {
        return Some("a rule's violations are missing or malformed");
    }
    let unwaived = || {
        rule["violations"]
            .as_array()
            .is_some_and(|violations| violations.iter().all(|v| v.get("waived_by").is_none()))
    };
    (builtin && !(rule["severity"] == "hold" && rule["outcome"] == "hold" && unwaived()))
        .then_some("a built-in rule is not a hold that no waiver covers")
}

fn lists_defect(document: &Value) -> Option<&'static str> {
    for member in ["expired_waivers", "unused_waivers"] {
        let whole = document[member].as_array().is_some_and(|waivers| {
            waivers.iter().all(|waiver| {
                waiver["index"].is_u64()
                    && waiver["rule"].is_string()
                    && waiver["owner"].is_string()
                    && is_date(&waiver["expires"])
            })
        });
        if !whole {
            return Some("a waiver list is missing or malformed");
        }
    }
    let notes = document["notes"].as_array().is_some_and(|notes| {
        notes.iter().all(|note| {
            is_name(&note["code"])
                && note["detail"].is_string()
                && optional_strings(note, &["path"])
        })
    });
    (!notes).then_some("`notes` is missing or malformed")
}

/// Why `document` is not a whole `forge-warden.verdict.v2`; `None` when it is.
///
/// This says nothing about which stop the verdict is for: the caller compares the seals with
/// the inputs it wrote and the outcome with the judge's exit code. The hook reads a document
/// by the version it names ([`defect_in`]); this is the v2 reading the tests ask about.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the hook reads by the version a document names")
)]
pub(crate) fn defect(document: &Value) -> Option<&'static str> {
    defect_in(document, Version::V2)
}

/// Why `document` is not a whole verdict of `version`; `None` when it is.
pub(crate) fn defect_in(document: &Value, version: Version) -> Option<&'static str> {
    if !document.is_object() {
        return Some("it is not a JSON object");
    }
    if document["schema"] != version.schema() {
        return Some(match version {
            Version::V2 => "it does not name the schema forge-warden.verdict.v2",
            Version::V3 => "it does not name the schema forge-warden.verdict.v3",
        });
    }
    if !(document["tool"]["name"] == "forge-warden" && document["tool"]["version"].is_string()) {
        return Some("`tool` does not name forge-warden and its version");
    }
    if !is_date(&document["evaluated_on"]) {
        return Some("`evaluated_on` is not a date");
    }
    if let Some(defect) = seals_defect(&document["seals"], version) {
        return Some(defect);
    }
    // The outcome and the exit triple that belongs to it.
    let exit = &document["exit"];
    let consistent = OUTCOMES.iter().any(|(verdict, code, name, action)| {
        document["verdict"] == *verdict
            && exit["code"] == *code
            && exit["name"] == *name
            && exit["action"] == *action
    });
    if !consistent {
        return Some("`verdict` and `exit` are missing or do not belong together");
    }
    let Some(rules) = document["rules"]
        .as_array()
        .filter(|rules| !rules.is_empty())
    else {
        return Some("`rules` is missing or empty");
    };
    if let Some(defect) = rules.iter().find_map(|rule| rule_defect(rule, version)) {
        return Some(defect);
    }
    // The summary counts the rules beside it.
    let count = |outcome: &str| {
        rules
            .iter()
            .filter(|rule| rule["outcome"] == outcome)
            .count()
    };
    let summary = &document["summary"];
    let says = |member: &str| {
        summary[member]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
    };
    if !(summary["files"].is_u64()
        && says("rules") == Some(rules.len())
        && says("passed") == Some(count("pass"))
        && says("waived") == Some(count("waived"))
        && says("held") == Some(count("hold"))
        && says("failed") == Some(count("fail")))
    {
        return Some("`summary` is missing or does not count the rules");
    }
    lists_defect(document)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A whole verdict of `outcome`, as forge-warden 0.2.0 writes one, sealed to the three
    /// digests given: a passed rule always, a held one unless it passes, a failed one if it
    /// fails.
    pub(crate) fn whole(outcome: &str, policy: &str, patch: &str, ranker: &str) -> Value {
        let (_, code, name, action) = OUTCOMES
            .iter()
            .find(|known| known.0 == outcome)
            .copied()
            .expect("a known outcome");
        let rule = |id: &str, outcome: &str, signal: &str, path: &str| {
            json!({"id": id, "kind": "forbid_signal", "severity": "hold", "outcome": outcome,
                   "why": format!("why {id}"),
                   "violations": [{"signal": signal, "path": path, "message": "m"}]})
        };
        let mut rules = vec![
            json!({"id": "size", "kind": "max_score", "severity": "hold",
                                    "outcome": "pass", "why": "why size", "violations": []}),
        ];
        if outcome != "pass" {
            rules.push(rule("keep-the-tests", "hold", "test_removed", "tests/a.rs"));
        }
        if outcome == "fail" {
            rules.push(rule("no-secrets", "fail", "secret_like", "src/k.rs"));
        }
        let mut document = json!({
            "schema": SCHEMA,
            "tool": {"name": "forge-warden", "version": "0.2.0"},
            "evaluated_on": "2026-11-14",
            "seals": {
                "policy_sha256": policy,
                "rank_sha256": "3".repeat(64),
                "patch_sha256": patch,
                "patch_facts_sha256": null,
                "provenance": "produced",
                "ranked_by": {"version": "deep-diff-forge 0.9.0", "sha256": ranker},
                "git": {"path": "/usr/bin/git", "version": "git version 2.56.0",
                        "sha256": "4".repeat(64), "admission": "approved",
                        "allowlist_sha256": "5".repeat(64)},
            },
            "verdict": outcome,
            "exit": {"code": code, "name": name, "action": action},
            "summary": {},
            "rules": rules,
            "expired_waivers": [],
            "unused_waivers": [],
            "notes": [],
        });
        recount(&mut document);
        document
    }

    /// Make `summary` count the rules of `document` again, after a test changed one.
    pub(crate) fn recount(document: &mut Value) {
        let rules = document["rules"].as_array().cloned().unwrap_or_default();
        let count = |outcome: &str| rules.iter().filter(|r| r["outcome"] == outcome).count();
        document["summary"] = json!({
            "files": 1, "rules": rules.len(), "passed": count("pass"),
            "waived": count("waived"), "held": count("hold"), "failed": count("fail"),
        });
    }

    fn sample(outcome: &str) -> Value {
        whole(outcome, &"1".repeat(64), &"2".repeat(64), &"6".repeat(64))
    }

    #[test]
    fn a_whole_verdict_of_each_outcome_has_no_defect() {
        for outcome in ["pass", "hold", "fail"] {
            assert_eq!(defect(&sample(outcome)), None, "{outcome}");
        }
        // what the schema leaves open, and what this reader leaves open on purpose
        let mut open = sample("hold");
        open["a_later_member"] = json!(1);
        open["seals"]["git"] = Value::Null;
        open["seals"]["patch_facts_sha256"] = json!("7".repeat(64));
        open["seals"]["ranked_by"]["sha256"] = Value::Null;
        open["seals"]["claims"] =
            json!({"verified": false, "base": "abcd", "head": "0f".repeat(32)});
        open["rules"][1]["outcome"] = json!("waived");
        open["rules"][1]["violations"][0] = json!({
            "message": "m", "path": "p", "signal": "s", "waived_by": 0, "evidence_truncated": true,
            "evidence": [{"address": "8".repeat(64), "side": "old", "line": 1}],
        });
        open["verdict"] = json!("pass");
        open["exit"] = json!({"code": 0, "name": "pass", "action": "proceed"});
        recount(&mut open);
        open["rules"][0]["kind"] = json!("a_kind_of_a_later_version");
        open["notes"] = json!([{"code": "a_later_code", "detail": "d", "path": "p"}]);
        open["unused_waivers"] =
            json!([{"index": 0, "rule": "size", "owner": "@o", "expires": "2027-01-01"}]);
        assert_eq!(defect(&open), None);
        let mut unpinned = sample("pass");
        unpinned["seals"]["git"]["admission"] = json!("unpinned");
        unpinned["seals"]["git"]["allowlist_sha256"] = Value::Null;
        assert_eq!(defect(&unpinned), None);
    }

    /// Review V4 round 3, VD4R3-M-1: each thing the schema fixes is taken away or bent on its
    /// own, and each time the document is no verdict. The reviewer's document did five of
    /// these at once and passed.
    #[test]
    fn every_member_the_schema_fixes_is_needed() {
        type Bend = fn(&mut Value);
        let bends: &[(&str, Bend)] = &[
            ("no tool", |d| d["tool"] = Value::Null),
            ("another tool", |d| d["tool"]["name"] = json!("x")),
            ("a date that is not one", |d| {
                d["evaluated_on"] = json!("not-a-date");
            }),
            ("no patch facts member", |d| {
                d["seals"]
                    .as_object_mut()
                    .unwrap()
                    .remove("patch_facts_sha256");
            }),
            ("no git member", |d| {
                d["seals"].as_object_mut().unwrap().remove("git");
            }),
            ("git without its digest", |d| {
                d["seals"]["git"]["sha256"] = json!("x");
            }),
            ("approved git without an allowlist", |d| {
                d["seals"]["git"]["allowlist_sha256"] = Value::Null;
            }),
            ("an empty ranked_by", |d| {
                d["seals"]["ranked_by"] = json!({});
            }),
            ("a short seal", |d| d["seals"]["rank_sha256"] = json!("abc")),
            ("an uppercase seal", |d| {
                d["seals"]["policy_sha256"] = json!("A".repeat(64));
            }),
            ("another provenance", |d| {
                d["seals"]["provenance"] = json!("guessed");
            }),
            ("a verified claim", |d| {
                d["seals"]["claims"] = json!({"verified": true});
            }),
            ("an unknown verdict", |d| d["verdict"] = json!("maybe")),
            ("the exit name of another outcome", |d| {
                d["exit"]["name"] = json!("policy_fail");
            }),
            ("the exit action of another outcome", |d| {
                d["exit"]["action"] = json!("change_input");
            }),
            ("the exit code of another outcome", |d| {
                d["exit"]["code"] = json!(7);
            }),
            ("no exit", |d| d["exit"] = Value::Null),
            ("an empty summary", |d| d["summary"] = json!({})),
            ("a summary that counts other rules", |d| {
                d["summary"]["passed"] = json!(9);
            }),
            ("no rules", |d| {
                d["rules"] = json!([]);
                recount(d);
            }),
            ("a rule id that is not one", |d| {
                d["rules"][0]["id"] = json!("Not An Id");
            }),
            ("a rule with no kind", |d| {
                d["rules"][0].as_object_mut().unwrap().remove("kind");
            }),
            ("a rule severity that is not one", |d| {
                d["rules"][0]["severity"] = json!("pass");
            }),
            ("a rule outcome that is not one", |d| {
                d["rules"][0]["outcome"] = json!("maybe");
                recount(d);
            }),
            ("violations that are not a list", |d| {
                d["rules"][0]["violations"] = json!({});
            }),
            ("a violation with no message", |d| {
                d["rules"][0]["violations"] = json!([{}]);
            }),
            ("no notes", |d| d["notes"] = Value::Null),
            ("a note with no detail", |d| {
                d["notes"] = json!([{"code": "x"}]);
            }),
            ("a waiver with no owner", |d| {
                d["expired_waivers"] = json!([{"index": 0, "rule": "r", "expires": "2026-01-01"}]);
            }),
            ("not an object", |d| *d = json!([1])),
            ("another schema", |d| {
                d["schema"] = json!("forge-warden.verdict.v1");
            }),
        ];
        for (what, bend) in bends {
            let mut bent = sample("pass");
            bend(&mut bent);
            assert!(defect(&bent).is_some(), "{what} was accepted");
        }
    }

    /// A mutation run on the hooks (2026-11-14) left six changes in this file alive. Each form
    /// below is two conditions, and the tests bent both at once: a rule id that neither starts
    /// with a letter nor is made of words, a `ranked_by` with nothing in it. Each case here
    /// breaks one condition and keeps the other.
    #[test]
    fn a_form_of_two_conditions_is_refused_when_either_fails() {
        type Bend = fn(&mut Value);
        let bends: &[(&str, Bend)] = &[
            ("a rule id with an empty word after a good start", |d| {
                d["rules"][0]["id"] = json!("a--b");
            }),
            ("a rule id of good words that starts with a digit", |d| {
                d["rules"][0]["id"] = json!("9-a");
            }),
            (
                "a rule kind with an uppercase letter after a good start",
                |d| {
                    d["rules"][0]["kind"] = json!("max_Score");
                },
            ),
            (
                "a rule kind of good characters that starts with a digit",
                |d| {
                    d["rules"][0]["kind"] = json!("9max");
                },
            ),
            ("a note code that starts with an underscore", |d| {
                d["notes"] = json!([{"code": "_x", "detail": "d"}]);
            }),
            ("a violation whose path is not a string", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "path": 5}]);
            }),
            ("a violation whose signal is not a string", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "signal": ["s"]}]);
            }),
            ("a note whose path is not a string", |d| {
                d["notes"] = json!([{"code": "x", "detail": "d", "path": 1}]);
            }),
            ("ranked_by with a digest and no version", |d| {
                d["seals"]["ranked_by"] = json!({"sha256": "6".repeat(64)});
            }),
            (
                "ranked_by with a version and a digest that is not one",
                |d| {
                    d["seals"]["ranked_by"] = json!({"version": "v", "sha256": "x"});
                },
            ),
            ("an unpinned git that names an allowlist", |d| {
                d["seals"]["git"]["admission"] = json!("unpinned");
            }),
            ("a git of another admission with no allowlist", |d| {
                d["seals"]["git"]["admission"] = json!("refused");
                d["seals"]["git"]["allowlist_sha256"] = Value::Null;
            }),
        ];
        for (what, bend) in bends {
            let mut bent = sample("pass");
            bend(&mut bent);
            assert!(defect(&bent).is_some(), "{what} was accepted");
        }
        // the forms themselves, with each optional part there, are accepted
        let controls: &[(&str, Bend)] = &[
            ("a note with a path", |d| {
                d["notes"] = json!([{"code": "x_1", "detail": "d", "path": "p"}]);
            }),
            ("a violation with a path and a signal", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "path": "p", "signal": "s"}]);
            }),
            ("ranked_by with no digest", |d| {
                d["seals"]["ranked_by"] = json!({"version": "v", "sha256": null});
            }),
            ("an unpinned git with no allowlist", |d| {
                d["seals"]["git"]["admission"] = json!("unpinned");
                d["seals"]["git"]["allowlist_sha256"] = Value::Null;
            }),
        ];
        for (what, bend) in controls {
            let mut bent = sample("pass");
            bend(&mut bent);
            assert_eq!(defect(&bent), None, "{what} was refused");
        }
        // what is no object is no Git admission at all, and is said to be missing
        let mut bent = sample("pass");
        bent["seals"]["git"] = json!("git");
        assert_eq!(defect(&bent), Some("`seals.git` is missing"));
    }

    /// Review V4 round 4, VD4R4-M-1: a member the schema makes optional has the schema's form
    /// when it is present. A document with four of these malformed, beside a waived rule, was a
    /// pass.
    #[test]
    fn an_optional_member_that_is_present_has_the_schemas_form() {
        type Bend = fn(&mut Value);
        let bends: &[(&str, Bend)] = &[
            ("a claimed base that is no commit", |d| {
                d["seals"]["claims"] = json!({"verified": false, "base": "NOT-HEX"});
            }),
            ("a claimed head that is too short", |d| {
                d["seals"]["claims"] = json!({"verified": false, "head": "abc"});
            }),
            ("evidence that is not a list", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "evidence": "x"}]);
            }),
            ("a citation with no address", |d| {
                d["rules"][0]["violations"] =
                    json!([{"message": "m", "evidence": [{"side": "new", "line": 1}]}]);
            }),
            ("a citation on no side", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "evidence": [
                    {"address": "8".repeat(64), "side": "both", "line": 1}]}]);
            }),
            ("a citation of line 0", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "evidence": [
                    {"address": "8".repeat(64), "side": "new", "line": 0}]}]);
            }),
            ("evidence_truncated that is false", |d| {
                d["rules"][0]["violations"] =
                    json!([{"message": "m", "evidence_truncated": false}]);
            }),
            ("a waiver index below zero", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "waived_by": -1}]);
            }),
        ];
        for (what, bend) in bends {
            let mut bent = sample("pass");
            bend(&mut bent);
            assert!(defect(&bent).is_some(), "{what} was accepted");
        }
    }

    /// Every property the schema names, in every object, against the code that reads it. Kept
    /// as a table so that a member has a place to be missed from: the list was taken from
    /// `schemas/verdict.v2.schema.json` of forge-warden 0.2.0.
    #[test]
    fn every_member_the_schema_names_is_read() {
        const NAMED: &[(&str, &[&str])] = &[
            (
                "verdict",
                &[
                    "schema",
                    "tool",
                    "evaluated_on",
                    "seals",
                    "verdict",
                    "exit",
                    "summary",
                    "rules",
                    "expired_waivers",
                    "unused_waivers",
                    "notes",
                ],
            ),
            ("tool", &["name", "version"]),
            (
                "seals",
                &[
                    "policy_sha256",
                    "rank_sha256",
                    "patch_sha256",
                    "patch_facts_sha256",
                    "provenance",
                    "ranked_by",
                    "git",
                    "claims",
                ],
            ),
            ("ranked_by", &["version", "sha256"]),
            (
                "git",
                &["path", "version", "sha256", "admission", "allowlist_sha256"],
            ),
            ("claims", &["base", "head", "verified"]),
            ("exit", &["code", "name", "action"]),
            (
                "summary",
                &["files", "rules", "passed", "waived", "held", "failed"],
            ),
            (
                "rule",
                &["id", "kind", "severity", "outcome", "why", "violations"],
            ),
            (
                "violation",
                &[
                    "path",
                    "message",
                    "signal",
                    "evidence",
                    "evidence_truncated",
                    "waived_by",
                ],
            ),
            ("citation", &["address", "side", "line"]),
            ("waiver_note", &["index", "rule", "owner", "expires"]),
            ("note", &["code", "path", "detail"]),
        ];
        let source = include_str!("verdict_v2.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the module has code before its tests");
        for (object, members) in NAMED {
            for member in *members {
                assert!(
                    code.contains(&format!("\"{member}\"")),
                    "`{object}.{member}` is named by the schema and read nowhere"
                );
            }
        }
    }

    /// The reviewer's document: five of those at once, and it passed.
    #[test]
    fn the_reviewers_document_is_no_verdict() {
        let mut theirs = sample("pass");
        theirs["evaluated_on"] = json!("not-a-date");
        for member in ["patch_facts_sha256", "git"] {
            theirs["seals"].as_object_mut().unwrap().remove(member);
        }
        theirs["seals"]["ranked_by"] = json!({});
        theirs["summary"] = json!({});
        theirs["rules"] = json!([]);
        theirs["exit"]["name"] = json!("policy_fail");
        theirs["exit"]["action"] = json!("change_input");
        assert!(defect(&theirs).is_some());
    }

    // `forge-warden.verdict.v3` (S3.21). The v2 tests above are as they were.

    /// forge-warden's own v3 document: `fixtures/demo.verdict.json` on its main branch at
    /// commit fbd621e0e4c3183e7cb9d9c949dac942b8fb0dd8 (the file last changed in dd05b45b),
    /// copied byte for byte. An advisory verdict: a supplied ranking, an unpinned Git, a fail
    /// with a failing rule, four held ones and a note.
    pub(crate) const FORGE_WARDEN_V3_DEMO: &str =
        include_str!("../tests/fixtures/forge-warden-main-fbd621e0/demo.verdict.json");

    /// A built-in rule as forge-warden's main branch writes one (`crates/forge-warden-core/
    /// src/builtin.rs` at fbd621e0: `builtin:binary-on-executable`, kind `builtin`, a hold, a
    /// violation with a path and a message and nothing else).
    pub(crate) fn builtin_rule() -> Value {
        json!({"id": "builtin:binary-on-executable", "kind": "builtin", "severity": "hold",
               "outcome": "hold",
               "why": "an executable or a symlink sent as a binary section hides its lines from every signal rule, and the shell can still run it; a person reads it before it merges",
               "violations": [{"path": "tools/run",
                               "message": "a binary section on an executable file (100755)"}]})
    }

    /// Bound claims as forge-warden's main branch writes them (`crates/forge-warden-cli/tests/
    /// authority_mode.rs` at fbd621e0, `bound_mode_verifies_the_claims_only_through_the_
    /// comparison`), with ids of `digits` hex digits.
    pub(crate) fn bound_claims(digits: usize) -> Value {
        json!({"base": "a".repeat(digits), "head": "b".repeat(digits),
               "merge_base": "c".repeat(digits), "verified": true})
    }

    /// [`whole`] under the v3 schema name: what v3 keeps of v2 is the same.
    pub(crate) fn whole_v3(outcome: &str, policy: &str, patch: &str, ranker: &str) -> Value {
        let mut document = whole(outcome, policy, patch, ranker);
        document["schema"] = json!(SCHEMA_V3);
        document
    }

    fn v3(document: &Value) -> Option<&'static str> {
        defect_in(document, Version::V3)
    }

    /// A v3 hold with what v3 adds: a built-in rule last, bound claims of SHA-1 ids.
    fn sample_v3() -> Value {
        let mut document = whole_v3("hold", &"1".repeat(64), &"2".repeat(64), &"6".repeat(64));
        document["rules"]
            .as_array_mut()
            .unwrap()
            .push(builtin_rule());
        document["seals"]["claims"] = bound_claims(40);
        recount(&mut document);
        document
    }

    #[test]
    fn forge_wardens_own_v3_document_is_a_whole_v3_and_no_v2() {
        let demo: Value = serde_json::from_str(FORGE_WARDEN_V3_DEMO).unwrap();
        assert_eq!(Version::named_by(&demo), Some(Version::V3));
        assert_eq!(v3(&demo), None);
        assert_eq!(
            defect(&demo),
            Some("it does not name the schema forge-warden.verdict.v2")
        );
        // the other way round: a v2 document is no v3 one
        assert_eq!(
            v3(&sample("pass")),
            Some("it does not name the schema forge-warden.verdict.v3")
        );
        assert_eq!(Version::named_by(&sample("pass")), Some(Version::V2));
        for other in [
            json!({"schema": "forge-warden.verdict.v4"}),
            json!({}),
            json!(3),
        ] {
            assert_eq!(Version::named_by(&other), None, "{other}");
        }
        for outcome in ["pass", "hold", "fail"] {
            let document = whole_v3(outcome, &"1".repeat(64), &"2".repeat(64), &"6".repeat(64));
            assert_eq!(v3(&document), None, "{outcome}");
        }
    }

    /// What v3 adds, each in the form forge-warden writes it, is read; none of it is v2's. And
    /// what v3 leaves open, like v2, is left open on purpose: a member it does not know, a rule
    /// kind and a note code that are new lowercase names.
    #[test]
    fn what_v3_adds_is_read_in_its_own_form_and_only_in_v3() {
        let mut added = sample_v3();
        assert_eq!(v3(&added), None);
        added["seals"]["claims"] = bound_claims(64);
        assert_eq!(v3(&added), None);
        // the unverified claims of v2, unchanged in v3
        for claims in [
            json!({"verified": false}),
            json!({"verified": false, "base": "abcd", "head": "0f".repeat(32)}),
        ] {
            added["seals"]["claims"] = claims;
            assert_eq!(v3(&added), None);
        }
        added["notes"] = json!([{"code": "claims_unverified", "detail": "d"}]);
        assert_eq!(v3(&added), None);
        // open on purpose
        let mut open = sample_v3();
        open["a_later_member"] = json!(1);
        open["seals"]["claims"]["a_later_member"] = json!(1);
        open["rules"][0]["kind"] = json!("a_kind_of_a_later_version");
        open["notes"] = json!([{"code": "a_later_code", "detail": "d"}]);
        assert_eq!(v3(&open), None);
        // under the v2 name, what v3 adds is not read
        let as_v2 = |change: fn(&mut Value)| {
            let mut document = sample("hold");
            change(&mut document);
            recount(&mut document);
            defect(&document)
        };
        assert_eq!(
            as_v2(|d| d["seals"]["claims"] = bound_claims(40)),
            Some("`seals.claims` is not an unverified claim of two commits")
        );
        assert!(as_v2(|d| d["rules"].as_array_mut().unwrap().push(builtin_rule())).is_some());
    }

    /// The table of [`every_member_the_schema_fixes_is_needed`] for v3: each thing the v3
    /// schema fixes (at forge-warden fbd621e0) is taken away or bent on its own, and each time
    /// the document is no v3 verdict. The first part is what v3 keeps of v2; the second is
    /// what v3 adds, each condition of a form of two broken while the other holds.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one table: each thing the v3 schema fixes, bent on its own, has its row"
    )]
    fn every_member_the_v3_schema_fixes_is_needed() {
        type Bend = fn(&mut Value);
        /// The index of the last rule: the built-in one.
        fn last(d: &Value) -> usize {
            d["rules"].as_array().unwrap().len() - 1
        }
        let bends: &[(&str, Bend)] = &[
            ("not an object", |d| *d = json!([1])),
            ("the v2 schema", |d| d["schema"] = json!(SCHEMA)),
            ("a later schema", |d| {
                d["schema"] = json!("forge-warden.verdict.v4");
            }),
            ("no schema", |d| {
                d.as_object_mut().unwrap().remove("schema");
            }),
            ("no tool", |d| d["tool"] = Value::Null),
            ("another tool", |d| d["tool"]["name"] = json!("x")),
            ("a tool with no version", |d| {
                d["tool"]["version"] = json!(1);
            }),
            ("a date that is not one", |d| {
                d["evaluated_on"] = json!("2026-1-01");
            }),
            ("no seals", |d| d["seals"] = Value::Null),
            ("no patch facts member", |d| {
                d["seals"]
                    .as_object_mut()
                    .unwrap()
                    .remove("patch_facts_sha256");
            }),
            ("no git member", |d| {
                d["seals"].as_object_mut().unwrap().remove("git");
            }),
            ("git without its digest", |d| {
                d["seals"]["git"]["sha256"] = json!("x");
            }),
            ("git without its path", |d| {
                d["seals"]["git"]["path"] = Value::Null;
            }),
            ("approved git without an allowlist", |d| {
                d["seals"]["git"]["allowlist_sha256"] = Value::Null;
            }),
            ("an unpinned git that names an allowlist", |d| {
                d["seals"]["git"]["admission"] = json!("unpinned");
            }),
            ("an empty ranked_by", |d| {
                d["seals"]["ranked_by"] = json!({});
            }),
            ("a short seal", |d| d["seals"]["rank_sha256"] = json!("abc")),
            ("an uppercase seal", |d| {
                d["seals"]["policy_sha256"] = json!("A".repeat(64));
            }),
            ("another provenance", |d| {
                d["seals"]["provenance"] = json!("guessed");
            }),
            ("an unknown verdict", |d| d["verdict"] = json!("maybe")),
            ("the exit name of another outcome", |d| {
                d["exit"]["name"] = json!("policy_fail");
            }),
            ("the exit action of another outcome", |d| {
                d["exit"]["action"] = json!("proceed");
            }),
            ("the exit code of another outcome", |d| {
                d["exit"]["code"] = json!(7);
            }),
            ("no exit", |d| d["exit"] = Value::Null),
            ("an empty summary", |d| d["summary"] = json!({})),
            ("a summary that counts other rules", |d| {
                d["summary"]["held"] = json!(9);
            }),
            ("a summary with no files", |d| {
                d["summary"]["files"] = json!(-1);
            }),
            ("no rules", |d| {
                d["rules"] = json!([]);
                recount(d);
            }),
            ("a rule id that is not one", |d| {
                d["rules"][0]["id"] = json!("Not An Id");
            }),
            ("a rule with no kind", |d| {
                d["rules"][0].as_object_mut().unwrap().remove("kind");
            }),
            ("a rule severity that is not one", |d| {
                d["rules"][0]["severity"] = json!("pass");
            }),
            ("a rule outcome that is not one", |d| {
                d["rules"][0]["outcome"] = json!("maybe");
                recount(d);
            }),
            ("a rule with no reason", |d| {
                d["rules"][0].as_object_mut().unwrap().remove("why");
            }),
            ("violations that are not a list", |d| {
                d["rules"][0]["violations"] = json!({});
            }),
            ("a violation with no message", |d| {
                d["rules"][0]["violations"] = json!([{}]);
            }),
            ("evidence that is not a list", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "evidence": "x"}]);
            }),
            ("a citation of line 0", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "evidence": [
                    {"address": "8".repeat(64), "side": "new", "line": 0}]}]);
            }),
            ("evidence_truncated that is false", |d| {
                d["rules"][0]["violations"] =
                    json!([{"message": "m", "evidence_truncated": false}]);
            }),
            ("a waiver index below zero", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "waived_by": -1}]);
            }),
            ("no notes", |d| d["notes"] = Value::Null),
            ("a note with no detail", |d| {
                d["notes"] = json!([{"code": "claims_unverified"}]);
            }),
            ("a note code that is no name", |d| {
                d["notes"] = json!([{"code": "Claims", "detail": "d"}]);
            }),
            ("a waiver with no owner", |d| {
                d["expired_waivers"] = json!([{"index": 0, "rule": "r", "expires": "2026-01-01"}]);
            }),
            ("no unused_waivers", |d| d["unused_waivers"] = Value::Null),
            // what v3 adds: the bound claims
            ("bound claims of 39 digits", |d| {
                d["seals"]["claims"] = bound_claims(39);
            }),
            ("bound claims of short ids", |d| {
                d["seals"]["claims"] = bound_claims(4);
            }),
            ("bound claims of mixed lengths", |d| {
                d["seals"]["claims"]["head"] = json!("b".repeat(64));
            }),
            ("bound claims with a merge base of another length", |d| {
                d["seals"]["claims"]["merge_base"] = json!("c".repeat(64));
            }),
            ("bound claims with no merge base", |d| {
                d["seals"]["claims"]
                    .as_object_mut()
                    .unwrap()
                    .remove("merge_base");
            }),
            ("bound claims with no base", |d| {
                d["seals"]["claims"].as_object_mut().unwrap().remove("base");
            }),
            ("bound claims with no head", |d| {
                d["seals"]["claims"].as_object_mut().unwrap().remove("head");
            }),
            ("bound claims with an uppercase id", |d| {
                d["seals"]["claims"]["head"] = json!("B".repeat(40));
            }),
            ("bound claims that are not verified", |d| {
                d["seals"]["claims"]["verified"] = json!(false);
            }),
            ("bound claims verified by a word", |d| {
                d["seals"]["claims"]["verified"] = json!("true");
            }),
            ("a verified claim of nothing", |d| {
                d["seals"]["claims"] = json!({"verified": true});
            }),
            ("an unverified claim with a merge base", |d| {
                d["seals"]["claims"] = json!({"verified": false, "merge_base": "c".repeat(40)});
            }),
            ("claims that are no object", |d| {
                d["seals"]["claims"] = json!(true);
            }),
            // what v3 adds: the built-in rule
            ("a built-in rule with a policy id", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("binary-on-executable");
            }),
            ("a policy rule with a built-in id", |d| {
                d["rules"][0]["id"] = json!("builtin:size");
            }),
            ("a built-in id with nothing after its prefix", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("builtin:");
            }),
            ("a built-in id that is prefixed twice", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("builtin:builtin:x");
            }),
            ("a built-in id whose policy id is malformed", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("builtin:Binary");
            }),
            ("a built-in rule of another kind's id prefix", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("built-in:x");
            }),
            ("a built-in rule that fails", |d| {
                let at = last(d);
                d["rules"][at]["severity"] = json!("fail");
            }),
            ("a built-in rule whose outcome fails", |d| {
                let at = last(d);
                d["rules"][at]["outcome"] = json!("fail");
                d["verdict"] = json!("fail");
                d["exit"] = json!({"code": 7, "name": "policy_fail", "action": "change_input"});
                recount(d);
            }),
            ("a built-in rule that passes", |d| {
                let at = last(d);
                d["rules"][at]["outcome"] = json!("pass");
                recount(d);
            }),
            ("a built-in rule that is waived", |d| {
                let at = last(d);
                d["rules"][at]["outcome"] = json!("waived");
                recount(d);
            }),
            ("a built-in violation a waiver covers", |d| {
                let at = last(d);
                d["rules"][at]["violations"][0]["waived_by"] = json!(0);
            }),
            ("a built-in violation with no message", |d| {
                let at = last(d);
                d["rules"][at]["violations"] = json!([{"path": "p"}]);
            }),
        ];
        assert_eq!(v3(&sample_v3()), None);
        for (what, bend) in bends {
            let mut bent = sample_v3();
            bend(&mut bent);
            assert!(v3(&bent).is_some(), "{what} was accepted");
        }
        // the controls of the forms of two: each with both conditions held is read
        let controls: &[(&str, Bend)] = &[
            ("a built-in rule with no violation", |d| {
                let at = last(d);
                d["rules"][at]["violations"] = json!([]);
            }),
            ("a built-in id of words and digits", |d| {
                let at = last(d);
                d["rules"][at]["id"] = json!("builtin:rule-2-of-3");
            }),
            ("bound claims of SHA-256 ids", |d| {
                d["seals"]["claims"] = bound_claims(64);
            }),
            ("a waived policy violation beside a built-in rule", |d| {
                d["rules"][0]["violations"] = json!([{"message": "m", "waived_by": 0}]);
            }),
        ];
        for (what, bend) in controls {
            let mut bent = sample_v3();
            bend(&mut bent);
            assert_eq!(v3(&bent), None, "{what} was refused");
        }
    }

    /// Every property the v3 schema names (`schemas/verdict.v3.schema.json` at forge-warden
    /// fbd621e0), and the values that have a meaning of their own, against the code that
    /// reads them.
    #[test]
    fn every_member_the_v3_schema_names_is_read() {
        const NAMED: &[(&str, &[&str])] = &[
            (
                "verdict",
                &[
                    "schema",
                    "tool",
                    "evaluated_on",
                    "seals",
                    "verdict",
                    "exit",
                    "summary",
                    "rules",
                    "expired_waivers",
                    "unused_waivers",
                    "notes",
                ],
            ),
            ("tool", &["name", "version"]),
            (
                "seals",
                &[
                    "policy_sha256",
                    "rank_sha256",
                    "patch_sha256",
                    "patch_facts_sha256",
                    "provenance",
                    "ranked_by",
                    "git",
                    "claims",
                ],
            ),
            ("ranked_by", &["version", "sha256"]),
            (
                "git",
                &["path", "version", "sha256", "admission", "allowlist_sha256"],
            ),
            ("claims", &["base", "head", "merge_base", "verified"]),
            ("exit", &["code", "name", "action"]),
            (
                "summary",
                &["files", "rules", "passed", "waived", "held", "failed"],
            ),
            (
                "rule",
                &["id", "kind", "severity", "outcome", "why", "violations"],
            ),
            ("built-in rule", &["builtin", "builtin:", "hold"]),
            (
                "violation",
                &[
                    "path",
                    "message",
                    "signal",
                    "evidence",
                    "evidence_truncated",
                    "waived_by",
                ],
            ),
            ("citation", &["address", "side", "line"]),
            ("waiver_note", &["index", "rule", "owner", "expires"]),
            ("note", &["code", "path", "detail"]),
            ("schema names", &[SCHEMA, SCHEMA_V3]),
        ];
        let source = include_str!("verdict_v2.rs");
        let code = source
            .split("#[cfg(test)]")
            .next()
            .expect("the module has code before its tests");
        for (object, members) in NAMED {
            for member in *members {
                assert!(
                    code.contains(&format!("\"{member}\"")),
                    "`{object}.{member}` is named by the v3 schema and read nowhere"
                );
            }
        }
    }
}
