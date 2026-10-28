#![no_main]
// Soundness of `reformat_only` (expert circle 2026-10-28, defect D1): the structural diff may call
// a change formatting only when the two sources are equal once every whitespace character is
// removed. Tokens, comments and string literals keep their non-whitespace bytes, so this holds
// for any sound token stream; a comment whose words were not tokens broke it.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    // `old NUL new`; without a NUL the two sides are identical.
    let (old, new) = input.split_once('\0').unwrap_or((input, input));
    let diff = deep_diff_forge_syntax::structural_diff(deep_diff_forge_syntax::Language::Rust, old, new);
    if diff.reformat_only() {
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(strip(old), strip(new), "reformat_only for a change that is not whitespace");
    }
});
