use deep_diff_forge_core::{Parallelism, ReviewFile};

/// Resolve a [`Parallelism`] setting into a concrete worker count, clamped to
/// at least 1 and at most the item count.
#[must_use]
pub fn resolve_workers(parallelism: Parallelism, items: usize) -> usize {
    let requested = match parallelism {
        Parallelism::Serial => 1,
        Parallelism::Fixed(n) => n as usize,
        Parallelism::Auto => {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        }
    };
    requested.max(1).min(items.max(1))
}

/// Split `n` items into `workers` contiguous, deterministic index ranges.
/// Empty ranges are omitted; the union of ranges is exactly `0..n` in order.
#[must_use]
pub fn contiguous_chunks(n: usize, workers: usize) -> Vec<(usize, usize)> {
    if n == 0 {
        return Vec::new();
    }
    let workers = workers.max(1).min(n);
    let base = n / workers;
    let remainder = n % workers;
    let mut chunks = Vec::with_capacity(workers);
    let mut start = 0;
    for w in 0..workers {
        let len = base + usize::from(w < remainder);
        if len == 0 {
            continue;
        }
        chunks.push((start, start + len));
        start += len;
    }
    chunks
}

/// Run `lane` over every file using bounded parallelism, returning results in
/// **deterministic input order** regardless of thread completion order.
///
/// Files are split into contiguous chunks (one per worker); each chunk is
/// computed on its own scoped thread and the chunk results are concatenated in
/// chunk order, so the output is reproducible across any worker count. `lane`
/// must be total (it returns a value for every file); a panicking lane drops
/// its chunk's results, which the caller can detect as a short output.
pub fn run_lane<T, F>(files: &[ReviewFile], parallelism: Parallelism, lane: F) -> Vec<T>
where
    F: Fn(usize, &ReviewFile) -> T + Sync,
    T: Send,
{
    if files.is_empty() {
        return Vec::new();
    }
    let workers = resolve_workers(parallelism, files.len());
    if workers <= 1 {
        return files.iter().enumerate().map(|(i, f)| lane(i, f)).collect();
    }

    let chunks = contiguous_chunks(files.len(), workers);
    let lane_ref = &lane;
    let collected: Vec<Vec<T>> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|&(start, end)| {
                scope.spawn(move || {
                    (start..end)
                        .map(|i| lane_ref(i, &files[i]))
                        .collect::<Vec<T>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    collected.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_diff_forge_patch::parse;

    fn files(n: usize) -> Vec<ReviewFile> {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..n {
            let _ = write!(s, "--- a/f{i}.rs\n+++ b/f{i}.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n");
        }
        parse(&s).unwrap()
    }

    #[test]
    fn serial_resolves_to_one_worker() {
        assert_eq!(resolve_workers(Parallelism::Serial, 100), 1);
    }

    #[test]
    fn fixed_resolves_to_requested() {
        assert_eq!(resolve_workers(Parallelism::Fixed(4), 100), 4);
    }

    #[test]
    fn fixed_clamped_to_item_count() {
        assert_eq!(resolve_workers(Parallelism::Fixed(8), 3), 3);
    }

    #[test]
    fn auto_is_at_least_one() {
        assert!(resolve_workers(Parallelism::Auto, 100) >= 1);
    }

    #[test]
    fn workers_never_zero_even_with_zero_items() {
        assert_eq!(resolve_workers(Parallelism::Fixed(0), 0), 1);
    }

    #[test]
    fn chunks_cover_all_indices_in_order() {
        let chunks = contiguous_chunks(10, 3);
        let mut covered = Vec::new();
        for (start, end) in chunks {
            covered.extend(start..end);
        }
        assert_eq!(covered, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn chunks_are_balanced() {
        // 10 items / 3 workers -> 4, 3, 3.
        let chunks = contiguous_chunks(10, 3);
        let lens: Vec<usize> = chunks.iter().map(|(s, e)| e - s).collect();
        assert_eq!(lens, vec![4, 3, 3]);
    }

    #[test]
    fn chunks_empty_for_zero_items() {
        assert!(contiguous_chunks(0, 4).is_empty());
    }

    #[test]
    fn chunks_clamp_workers_to_items() {
        let chunks = contiguous_chunks(2, 8);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn run_lane_serial_returns_input_order() {
        let fs = files(5);
        let out = run_lane(&fs, Parallelism::Serial, |i, _| i);
        assert_eq!(out, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn run_lane_parallel_returns_input_order() {
        let fs = files(20);
        let out = run_lane(&fs, Parallelism::Fixed(4), |i, _| i);
        assert_eq!(out, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn run_lane_is_deterministic_across_worker_counts() {
        let fs = files(50);
        let serial = run_lane(&fs, Parallelism::Serial, |_, f| f.path.clone());
        let p2 = run_lane(&fs, Parallelism::Fixed(2), |_, f| f.path.clone());
        let p7 = run_lane(&fs, Parallelism::Fixed(7), |_, f| f.path.clone());
        let auto = run_lane(&fs, Parallelism::Auto, |_, f| f.path.clone());
        assert_eq!(serial, p2);
        assert_eq!(serial, p7);
        assert_eq!(serial, auto);
    }

    #[test]
    fn run_lane_empty_is_empty() {
        let out = run_lane(&[], Parallelism::Auto, |i, _| i);
        assert!(out.is_empty());
    }

    #[test]
    fn run_lane_single_file() {
        let fs = files(1);
        let out = run_lane(&fs, Parallelism::Fixed(4), |i, _| i);
        assert_eq!(out, vec![0]);
    }

    #[test]
    fn run_lane_passes_correct_file_to_each_index() {
        let fs = files(6);
        let out = run_lane(&fs, Parallelism::Fixed(3), |i, f| (i, f.path.clone()));
        for (i, (idx, path)) in out.iter().enumerate() {
            assert_eq!(*idx, i);
            assert_eq!(path, &format!("f{i}.rs"));
        }
    }

    #[test]
    fn run_lane_more_workers_than_files() {
        let fs = files(3);
        let out = run_lane(&fs, Parallelism::Fixed(16), |i, _| i);
        assert_eq!(out, vec![0, 1, 2]);
    }
}
