"""Reassemble a deep-diff-forge.diagnostics.v0 document from the pages of `diff.diagnostics`.

Written from docs/reference/diagnostics.md alone ("Byte form" and "Paging"), with the standard
library, as a client without the Rust writer would: stdin holds one page (the JSON-RPC `result`)
per line; stdout gets the document's bytes. Exit 1 when the bytes do not hash to every page's
`page.document_sha256`. Used by crates/deep-diff-forge-cli/tests/daemon_parity.rs.
"""
import hashlib
import json
import sys

HIDDEN = ((0x00AD, 0x00AD), (0x034F, 0x034F), (0x061C, 0x061C), (0x115F, 0x1160), (0x17B4, 0x17B5),
          (0x180B, 0x180F), (0x200B, 0x200F), (0x202A, 0x202E), (0x2060, 0x206F), (0x3164, 0x3164),
          (0xFE00, 0xFE0F), (0xFEFF, 0xFEFF), (0xFFA0, 0xFFA0), (0xFFF0, 0xFFFB), (0x1BCA0, 0x1BCA3),
          (0x1D173, 0x1D17A), (0xE0000, 0xE0FFF))
NAMED = {'"': '\\"', "\\": "\\\\", "\n": "\\n", "\r": "\\r", "\t": "\\t", "\b": "\\b", "\f": "\\f"}
ORDER = {"tool": ("name", "version", "scorer_sha256"), "judge": ("state", "ranker_sha256"),
         "counts": ("files", "diagnostics", "agent", "person", "cut_files"), "cut": ("path", "rows_kept"),
         "diagnostic": ("key", "code", "label", "class", "path", "anchor", "side", "line", "occurrence",
                        "reading", "audience", "cases", "repairs", "not_accepted", "test_likely", "judge_sees")}


def string(text):
    out = []
    for c in text:
        n = ord(c)
        if c in NAMED:
            out.append(NAMED[c])
        elif n < 0x20 or 0x7F <= n <= 0x9F or n in (0x2028, 0x2029) or any(a <= n <= b for a, b in HIDDEN):
            units = [n] if n <= 0xFFFF else [0xD800 + ((n - 0x10000) >> 10), 0xDC00 + ((n - 0x10000) & 0x3FF)]
            out.extend("\\u%04x" % u for u in units)
        else:
            out.append(c)
    return '"' + "".join(out) + '"'


def value(v):
    if isinstance(v, str):
        return string(v)
    if v is None or isinstance(v, bool):
        return {None: "null", True: "true", False: "false"}[v]
    if isinstance(v, int):
        return str(v)
    return "[" + ", ".join(value(x) for x in v) + "]"


def obj(d, kind):
    return "{" + ", ".join(string(k) + ": " + value(d[k]) for k in ORDER[kind] if k in d) + "}"


def block(items, opening, closing):
    return opening + closing if not items else opening + "\n    " + ",\n    ".join(items) + "\n  " + closing


pages = [json.loads(line) for line in sys.stdin.read().splitlines() if line.strip()]
first, cut, diagnostics, texts = pages[0], [], [], {}
for page in pages:
    cut += page["cut"]
    diagnostics += page["diagnostics"]
    texts.update(page["texts"])
members = [("schema", string(first["schema"])), ("input_sha256", string(first["input_sha256"]))]
members += [(k, obj(first[k], k)) for k in ("tool", "judge", "counts")]
members.append(("cut", block([obj(c, "cut") for c in cut], "[", "]")))
members.append(("diagnostics", block([obj(d, "diagnostic") for d in diagnostics], "[", "]")))
ids = sorted(texts, key=lambda k: k.encode())
members.append(("texts", block([string(k) + ": " + string(texts[k]) for k in ids], "{", "}")))
document = ("{\n" + ",\n".join("  " + string(k) + ": " + v for k, v in members) + "\n}\n").encode()
sys.stdout.buffer.write(document)
digest = hashlib.sha256(document).hexdigest()
sys.exit(0 if all(p["page"]["document_sha256"] == digest for p in pages) else 1)
