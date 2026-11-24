"""A stand-in for forge-warden, for scripts/contract_matrix.sh (F4.a).

Run as `judge.py <row directory> <the arguments the hook gives the judge>`. It reads its answer
from `<row>/answer.json`, records its pid (`pid`), its arguments (`judge.log`) and the patch it
was given (`judged.patch`) in the row directory, and then answers as forge-warden does: a whole
verdict document sealed to the policy, the patch and the ranker it was given, and the exit code
that belongs to the outcome (0 pass, 7 fail, 8 hold).

answer.json: {"schema": "v2" | "v3" | a schema name this build does not read,
              "outcome": "pass" | "hold" | "fail", "exit": <int>, "notes": [...],
              "hang": true (sleeps until it is killed), "refuse": true (exit 4, no document),
              "when": [[word, {members}], ...] (the members of the first entry whose word the
              patch holds replace the answer's own: one answer per commit of a branch)}

The documents are forge-warden's own: v2 from tests/fixtures/forge-warden-0.2.0/<outcome>.json
(forge-warden 0.2.0), v3 from tests/fixtures/forge-warden-main-fbd621e0/demo.verdict.json with
the rules of the outcome kept, as the hook's tests make them (agent_hook.rs `verdict_v3`), and
bound claims in the seals as forge-warden's bound mode writes them.
"""

import hashlib
import json
import os
import pathlib
import shutil
import sys
import time

FIXTURES = pathlib.Path(__file__).resolve().parents[2] / "crates/deep-diff-forge-cli/tests/fixtures"
KEEP = {"pass": ["pass"], "hold": ["pass", "hold"], "fail": ["pass", "fail", "hold"]}
EXIT = {"pass": (0, "pass", "proceed"), "fail": (7, "policy_fail", "change_input"),
        "hold": (8, "policy_hold", "review_evidence")}


def value(args, flag):
    return args[args.index(flag) + 1] if flag in args[:-1] else None


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest() if path else None


def document(answer, policy, patch, ranker):
    outcome, schema = answer["outcome"], answer["schema"]
    seals = {"policy_sha256": digest(policy), "rank_sha256": digest(patch),
             "patch_sha256": digest(patch), "patch_facts_sha256": None, "provenance": "produced",
             "ranked_by": {"version": "stand-in", "sha256": digest(ranker)},
             "git": {"path": "/usr/bin/git", "version": "git version 2.56.0", "sha256": digest(policy),
                     "admission": "approved", "allowlist_sha256": digest(patch)}}
    if schema == "v2":
        doc = json.loads((FIXTURES / f"forge-warden-0.2.0/{outcome}.json").read_text())
        doc["seals"] = seals
        return doc
    demo = json.loads((FIXTURES / "forge-warden-main-fbd621e0/demo.verdict.json").read_text())
    rules = [r for r in demo["rules"] if r["outcome"] in KEEP[outcome]]
    count = lambda o: sum(r["outcome"] == o for r in rules)
    seals["claims"] = {"base": "a" * 40, "head": "b" * 40, "merge_base": "c" * 40, "verified": True}
    code, name, action = EXIT[outcome]
    return {"schema": demo["schema"] if schema == "v3" else schema, "tool": demo["tool"],
            "evaluated_on": demo["evaluated_on"], "seals": seals, "verdict": outcome,
            "exit": {"code": code, "name": name, "action": action},
            "summary": {"files": 4, "rules": len(rules), "passed": count("pass"),
                        "waived": count("waived"), "held": count("hold"), "failed": count("fail")},
            "rules": rules, "expired_waivers": [], "unused_waivers": [],
            "notes": answer.get("notes", [])}


def main():
    row, args = pathlib.Path(sys.argv[1]), sys.argv[2:]
    (row / "pid").write_text(str(os.getpid()))
    answer = json.loads((row / "answer.json").read_text())
    with open(row / "judge.log", "a", encoding="utf-8") as log:
        log.write(json.dumps(args) + "\n")
    policy, patch, ranker = value(args, "--policy"), value(args, "--patch"), value(args, "--ranker")
    if patch:
        shutil.copyfile(patch, row / "judged.patch")
        text = pathlib.Path(patch).read_bytes()
        for word, members in answer.get("when", []):
            if word.encode() in text:
                answer = {**answer, **members}
                break
    if answer.get("hang"):
        time.sleep(3600)
    if answer.get("refuse"):
        print("forge-warden: the policy is refused (stand-in)", file=sys.stderr)
        return 4
    print(json.dumps(document(answer, policy, patch, ranker), indent=2))
    return answer["exit"]


if __name__ == "__main__":
    sys.exit(main())
