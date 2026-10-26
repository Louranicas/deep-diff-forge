// Review council: One canonical changed-source digest reused by analysis and council; explicit absence and framed paths/bytes.
// Design/status: docs/REVIEW_COUNCIL.md#canonical-snapshot
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fcouncil--crates--deep-diff-forge-core--src--snapshot
//! One canonical changed-source snapshot digest for analysis and review consumers.
use crate::sha256_hex;
/// Hash sorted changed-path bytes with explicit absence/presence and length framing.
/// Callers validate path uniqueness and acquisition budgets before calling this pure helper.
/// This seals changed sources, not modes, external dependencies or an entire Git tree.
#[must_use]
pub fn changed_source_snapshot_id<'a>(
    sources: impl IntoIterator<Item = (&'a str, Option<&'a [u8]>)>,
) -> String {
    let mut sorted = sources.into_iter().collect::<Vec<_>>();
    sorted.sort_by_key(|(path, _)| *path);
    let mut framed = b"ddf.changed-source-snapshot.v1\0".to_vec();
    for (path, bytes) in sorted {
        frame(&mut framed, path.as_bytes());
        framed.push(u8::from(bytes.is_some()));
        if let Some(bytes) = bytes {
            frame(&mut framed, bytes);
        }
    }
    sha256_hex(&framed)
}
fn frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_is_stable_but_absence_and_boundaries_are_distinct() {
        assert_eq!(
            changed_source_snapshot_id([("a", Some(b"x".as_slice())), ("b", None)]),
            changed_source_snapshot_id([("b", None), ("a", Some(b"x".as_slice()))])
        );
        assert_ne!(
            changed_source_snapshot_id([("a", None)]),
            changed_source_snapshot_id([("a", Some(b"".as_slice()))])
        );
        assert_ne!(
            changed_source_snapshot_id([("ab", Some(b"c".as_slice()))]),
            changed_source_snapshot_id([("a", Some(b"bc".as_slice()))])
        );
    }
}
