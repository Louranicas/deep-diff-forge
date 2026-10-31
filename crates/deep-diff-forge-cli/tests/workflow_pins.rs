//! Every third-party action in every workflow is pinned to a full commit SHA, so a moved tag
//! or a compromised release of an action cannot change what CI or the release runs.

use std::path::Path;

/// The `uses:` targets of a workflow, with the line each is on.
fn uses(workflow: &str) -> Vec<(usize, &str)> {
    workflow
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let rest = line
                .trim_start()
                .strip_prefix("- ")
                .unwrap_or(line.trim_start());
            let target = rest.strip_prefix("uses:")?.trim();
            Some((i + 1, target.split_whitespace().next().unwrap_or_default()))
        })
        .collect()
}

/// A target that is local (`./…`) or `owner/repo[/path]@<40 lowercase hex>`.
fn pinned(target: &str) -> bool {
    if target.starts_with("./") {
        return true;
    }
    let Some((action, sha)) = target.split_once('@') else {
        return false;
    };
    action.contains('/')
        && sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn every_action_is_pinned_to_a_commit() {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows"
    ));
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "yml" && e != "yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (line, target) in uses(&text) {
            seen += 1;
            assert!(
                pinned(target),
                "{}:{line}: `{target}` is not pinned to a commit SHA",
                path.display()
            );
        }
    }
    assert!(
        seen > 30,
        "only {seen} `uses:` lines found: the reader is broken"
    );
}

#[test]
fn the_pin_reader_is_exact() {
    let sha = "24c54180a607b1449ed407dd24f251e4e9147c8d";
    assert!(pinned(&format!("actions/checkout@{sha}")));
    assert!(pinned(&format!("github/codeql-action/init@{sha}")));
    assert!(pinned("./.github/workflows/ci.yml"));
    for bad in [
        "actions/checkout@v4".to_owned(),
        "actions/checkout@main".to_owned(),
        format!("actions/checkout@{}", &sha[..39]),
        format!("actions/checkout@{sha}0"),
        format!("actions/checkout@{}", sha.to_uppercase()),
        format!("checkout@{sha}"),
        "actions/checkout".to_owned(),
    ] {
        assert!(!pinned(&bad), "{bad}");
    }
    let text = "steps:\n  - uses: a/b@x # c\n    with:\n  - name: n\n    uses: ./local\n";
    assert_eq!(uses(text), [(2, "a/b@x"), (5, "./local")]);
}
