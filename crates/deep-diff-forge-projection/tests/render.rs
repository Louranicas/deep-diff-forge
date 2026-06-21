//! Integration tests: project the on-disk fixture corpus through both layouts.

use deep_diff_forge_patch::parse;
use deep_diff_forge_projection::{
    Layout, ProjectionOptions, render, render_inline, render_side_by_side,
};

fn load(name: &str) -> String {
    let path = format!("{}/../../fixtures/patch/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn basic_inline_render_shows_change() {
    let files = parse(&load("basic.patch")).unwrap();
    let out = render_inline(&files);
    assert!(out.contains("modified  src/lib.rs"));
    assert!(out.contains("-     let x = 1;"));
    assert!(out.contains("+     let x = 2;"));
}

#[test]
fn basic_side_by_side_render_has_gutter() {
    let files = parse(&load("basic.patch")).unwrap();
    let out = render_side_by_side(&files, 40);
    assert!(out.contains(" | "));
    assert!(out.contains("modified  src/lib.rs"));
}

#[test]
fn new_file_inline_is_all_additions() {
    let files = parse(&load("new_file.patch")).unwrap();
    let out = render_inline(&files);
    assert!(out.contains("added  hello.txt"));
    assert!(out.contains("+ hello"));
    assert!(out.contains("+ world"));
}

#[test]
fn binary_file_renders_header_only() {
    let files = parse(&load("binary.patch")).unwrap();
    let inline = render_inline(&files);
    assert!(inline.contains("binary_changed  logo.png"));
    // no body rows for a binary file
    assert!(!inline.contains(" + "));
}

#[test]
fn render_dispatch_inline_equals_direct() {
    let files = parse(&load("basic.patch")).unwrap();
    let opts = ProjectionOptions {
        layout: Layout::Inline,
        side_width: 40,
    };
    assert_eq!(render(&files, opts), render_inline(&files));
}

#[test]
fn every_fixture_projects_without_panicking() {
    for name in [
        "basic.patch",
        "new_file.patch",
        "delete_file.patch",
        "rename.patch",
        "binary.patch",
        "no_newline.patch",
    ] {
        let files = parse(&load(name)).unwrap();
        let _ = render_inline(&files);
        let _ = render_side_by_side(&files, 50);
    }
}
