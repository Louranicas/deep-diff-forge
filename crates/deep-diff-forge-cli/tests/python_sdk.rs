//! The Python package in `sdk/python` carries the workspace version in both places it states
//! one, so a release cannot ship a client that names another version.

use std::path::Path;

fn read(relative: &str) -> String {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    std::fs::read_to_string(root.join(relative)).unwrap()
}

/// The quoted value of the first line that starts with `key = "`.
fn quoted<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let rest = line
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix('=')?
            .trim();
        rest.strip_prefix('"')?.strip_suffix('"')
    })
}

#[test]
fn the_python_package_carries_the_workspace_version() {
    let version = env!("CARGO_PKG_VERSION");
    let pyproject = read("sdk/python/pyproject.toml");
    assert_eq!(
        quoted(&pyproject, "version"),
        Some(version),
        "pyproject.toml"
    );
    assert_eq!(quoted(&pyproject, "name"), Some("deep-diff-forge"));
    assert!(pyproject.contains("py-modules = [\"deep_diff_forge\"]"));
    assert!(
        pyproject.contains("dependencies = []"),
        "standard library only"
    );
    let module = read("sdk/python/deep_diff_forge.py");
    assert_eq!(
        quoted(&module, "__version__"),
        Some(version),
        "deep_diff_forge.py"
    );
}

#[test]
fn the_version_reader_is_exact() {
    assert_eq!(
        quoted("name = \"a\"\nversion = \"1.2.3\"\n", "version"),
        Some("1.2.3")
    );
    assert_eq!(
        quoted("version_x = \"9\"\nversion=\"1\"\n", "version"),
        Some("1")
    );
    assert_eq!(quoted("version = 1\n", "version"), None);
}
