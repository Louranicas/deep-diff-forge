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
//! still be lowercase names). Everything else the schema fixes is fixed here.

use serde_json::Value;

/// The schema name every verdict carries.
pub(crate) const SCHEMA: &str = "forge-warden.verdict.v2";

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

fn seals_defect(seals: &Value) -> Option<&'static str> {
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
    if let Some(claims) = seals.get("claims")
        && !(claims.is_object() && claims["verified"] == false)
    {
        return Some("`seals.claims` is not an unverified claim");
    }
    None
}

fn rule_defect(rule: &Value) -> Option<&'static str> {
    if !(is_rule_id(&rule["id"])
        && is_name(&rule["kind"])
        && is_one_of(&rule["severity"], &["fail", "hold"])
        && is_one_of(&rule["outcome"], &["pass", "waived", "hold", "fail"])
        && rule["why"].is_string())
    {
        return Some(
            "a rule lacks its id, kind, severity, outcome or reason, or has one malformed",
        );
    }
    let whole = rule["violations"].as_array().is_some_and(|violations| {
        violations.iter().all(|violation| {
            violation["message"].is_string() && optional_strings(violation, &["path", "signal"])
        })
    });
    (!whole).then_some("a rule's violations are missing or malformed")
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
/// the inputs it wrote and the outcome with the judge's exit code.
pub(crate) fn defect(document: &Value) -> Option<&'static str> {
    if !document.is_object() {
        return Some("it is not a JSON object");
    }
    if document["schema"] != SCHEMA {
        return Some("it does not name the schema forge-warden.verdict.v2");
    }
    if !(document["tool"]["name"] == "forge-warden" && document["tool"]["version"].is_string()) {
        return Some("`tool` does not name forge-warden and its version");
    }
    if !is_date(&document["evaluated_on"]) {
        return Some("`evaluated_on` is not a date");
    }
    if let Some(defect) = seals_defect(&document["seals"]) {
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
    if let Some(defect) = rules.iter().find_map(rule_defect) {
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
        open["seals"]["claims"] = json!({"verified": false, "base": "abcd"});
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
}
