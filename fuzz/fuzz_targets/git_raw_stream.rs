#![no_main]
// DDF 0.10.0, task S3.32.c: a `git diff --raw -z --patch` stream as the Git modes read it
// (`StructuredDiff::parse` and `append_stream`): the records, the file names Git prints and the
// patch blocks they own. Seeds: three streams `scripts/fuzz_target.sh` writes (a change, an
// addition, a rename). Design/status: docs/TESTING_GOLD_STANDARD.md (fuzz targets).
//
// The property, for any byte string up to 256 KiB: no panic in the reading, in a second stream
// appended to the first (the input split at the place byte 0 names) or in sealing the bytes
// (`capture`); a stream that is read keeps its patch bytes as given, and each change owns, in
// order and without overlap, a part of them that starts a `diff --git` block; an append that is
// refused leaves the diff as it was.
use deep_diff_forge_patch::{GitChangeOrigin, StructuredDiff};
use libfuzzer_sys::fuzz_target;

/// The changes of `diff` own ordered, disjoint parts of its patch, each a `diff --git` block.
fn check(diff: &StructuredDiff) {
    let mut end = 0;
    for change in diff.changes() {
        let range = &change.patch_range;
        assert!(
            end <= range.start && range.start < range.end,
            "{range:?} after {end}"
        );
        assert!(
            range.end <= diff.patch_bytes().len(),
            "{range:?} past the patch"
        );
        assert!(
            diff.change_patch(change).starts_with(b"diff --git "),
            "{change:?}"
        );
        assert!(change.block_count >= 1, "{change:?}");
        end = range.end;
    }
    let _ = diff.capture();
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 256 * 1024 {
        return;
    }
    if let Ok(diff) = StructuredDiff::parse(data) {
        assert!(
            data.ends_with(diff.patch_bytes()),
            "the patch bytes are not the input's"
        );
        check(&diff);
    }
    let Some((&at, rest)) = data.split_first() else {
        return;
    };
    let (first, second) = rest.split_at(usize::from(at) * rest.len() / 256);
    let Ok(mut diff) = StructuredDiff::parse(first) else {
        return;
    };
    let before = diff.clone();
    match diff.append_stream(second, GitChangeOrigin::Untracked) {
        Ok(added) => assert!(added.iter().all(|c| c.origin == GitChangeOrigin::Untracked)),
        Err(_) => assert_eq!(diff, before, "a refused append changed the diff"),
    }
    check(&diff);
});
