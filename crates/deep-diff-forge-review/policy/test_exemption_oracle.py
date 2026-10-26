#!/usr/bin/python3
"""Unit tests for the exemption oracle's pure functions (council r15: the core was untested below
the fake runtime). Run: python3 -I crates/deep-diff-forge-review/policy/test_exemption_oracle.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exemption_oracle as oracle  # noqa: E402

FN = "fn f() -> &str {\n    _0 = const {alloc%s: &str};\n}\n"


def mir(order, numbers, contents):
    """A MIR text whose body references `numbers[0]` and defines allocations in `order`."""
    body = FN % numbers[0]
    defs = [f"alloc{numbers[i]} (size: 1, align: 1) {{\n    {contents[i]}\n}}\n" for i in order]
    return body + "\n" + "\n".join(defs)


class Canonical(unittest.TestCase):
    def test_renumbering_and_reordering_cancel_out(self):
        a = mir([0, 1], ["1", "2"], ["00 │ x  ╾alloc2╼", "01 │ y"])
        b = mir([1, 0], ["7", "3"], ["00 │ x  ╾alloc3╼", "01 │ y"])
        self.assertEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))

    def test_contents_still_count(self):
        a = mir([0, 1], ["1", "2"], ["00 │ x  ╾alloc2╼", "01 │ y"])
        b = mir([0, 1], ["1", "2"], ["00 │ x  ╾alloc2╼", "01 │ z"])
        self.assertNotEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))

    def test_cycles_are_order_independent(self):
        a = mir([0, 1], ["1", "2"], ["00 │ a ╾alloc2╼", "01 │ b ╾alloc1╼"])
        b = mir([1, 0], ["9", "4"], ["00 │ a ╾alloc4╼", "01 │ b ╾alloc9╼"])
        self.assertEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))
        c = mir([0, 1], ["1", "2"], ["00 │ a ╾alloc2╼", "01 │ c ╾alloc1╼"])
        self.assertNotEqual(oracle.canonical_mir(a), oracle.canonical_mir(c))

    def test_string_literals_are_data_not_references(self):
        a = 'fn f() {\n    _0 = const "alloc1";\n}\n'
        b = 'fn f() {\n    _0 = const "alloc2";\n}\n'
        self.assertNotEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))

    def test_identifiers_that_look_like_allocations_are_left_alone(self):
        # `fn alloc7` names no allocation: it must neither collapse with another name nor fail
        # (council r16: a sentinel let a candidate make a configuration unobservable).
        a = "fn alloc7() {\n}\n"
        b = "fn alloc8() {\n}\n"
        self.assertNotEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))
        self.assertEqual(oracle.canonical_mir(a), a + "\n")

    def test_a_quote_never_spans_lines(self):
        # A hex dump's ASCII column can show a lone `"`; it must not pair with a quote below it.
        a = mir([0], ["1"], ['00 │ 22 │ "'])
        b = mir([0], ["9"], ['00 │ 22 │ "'])
        self.assertEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))

    def test_empty_single_line_allocations(self):
        a = FN % "3" + "alloc3 (size: 0, align: 1) {}\n"
        b = FN % "8" + "alloc8 (size: 0, align: 1) {}\n"
        self.assertEqual(oracle.canonical_mir(a), oracle.canonical_mir(b))


def span(l1, c1, l2, c2):
    return {"start": {"line": l1, "column": c1}, "end": {"line": l2, "column": c2}}


class Neutralise(unittest.TestCase):
    def test_lines_are_kept(self):
        text = "fn f() -> u32 {\n    let a = 1;\n    a + 2\n}\n"
        out = oracle.neutralised(text, [span(2, 5, 3, 10)], "loop {}")
        self.assertEqual(out.count("\n"), text.count("\n"))
        self.assertIn("loop {}", out)
        self.assertNotIn("a + 2", out)

    def test_columns_are_kept_when_the_body_is_long_enough(self):
        text = "fn f() -> u32 { 1 + 2 + 3 } // tail"
        out = oracle.neutralised(text, [span(1, 17, 1, 26)], "loop {}")
        self.assertEqual(out.index("// tail"), text.index("// tail"))

    def test_nested_spans_are_dropped(self):
        outer, inner, other = span(1, 1, 9, 1), span(2, 1, 3, 1), span(10, 1, 11, 1)
        kept = oracle.outermost([("a.rs", outer), ("a.rs", inner), ("a.rs", other), ("b.rs", inner)])
        self.assertEqual(kept, [("a.rs", outer), ("a.rs", other), ("b.rs", inner)])


class Artifacts(unittest.TestCase):
    def test_lib_is_a_prefix_of_library_files_only(self):
        self.assertEqual(oracle.stem_of("/t/deps/libfoo-1a.rlib"), "foo-1a")
        self.assertEqual(oracle.stem_of("/t/deps/libv7.so"), "v7")
        self.assertEqual(oracle.stem_of("/t/deps/library-2b"), "library-2b")
        self.assertEqual(oracle.stem_of("/t/deps/liberty-3c"), "liberty-3c")


if __name__ == "__main__":
    unittest.main(verbosity=1)
