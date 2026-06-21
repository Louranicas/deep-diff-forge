use deep_diff_forge_core::ReviewDocument;

fn main() {
    let document = ReviewDocument::empty();
    println!(
        "deep-diff-forge bootstrap: {} review files loaded",
        document.files.len()
    );
}
