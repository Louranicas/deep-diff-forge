/// A language the semantic layer can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// Rust (tree-sitter-rust).
    Rust,
    /// Any extension the registry does not recognize.
    Unsupported,
}

impl Language {
    /// Friendly language name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Unsupported => "unsupported",
        }
    }

    /// Whether a tree-sitter grammar is available for this language.
    #[must_use]
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Rust)
    }
}

/// Detect a language from a file path's extension.
#[must_use]
pub fn detect_language(path: &str) -> Language {
    let ext = path.rsplit('.').next().filter(|e| *e != path);
    match ext {
        Some("rs") => Language::Rust,
        _ => Language::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_extension_detected() {
        assert_eq!(detect_language("src/lib.rs"), Language::Rust);
        assert_eq!(detect_language("main.rs"), Language::Rust);
    }

    #[test]
    fn unknown_extension_is_unsupported() {
        assert_eq!(detect_language("notes.txt"), Language::Unsupported);
        assert_eq!(detect_language("data.json"), Language::Unsupported);
    }

    #[test]
    fn no_extension_is_unsupported() {
        assert_eq!(detect_language("Makefile"), Language::Unsupported);
        assert_eq!(detect_language("README"), Language::Unsupported);
    }

    #[test]
    fn dotfile_without_extension_is_unsupported() {
        // ".gitignore" -> rsplit('.') yields "gitignore" then "", but the
        // filtered guard keeps the bare-name case unsupported.
        assert_eq!(detect_language(".gitignore"), Language::Unsupported);
    }

    #[test]
    fn path_with_dots_uses_last_extension() {
        assert_eq!(detect_language("a.b.c.rs"), Language::Rust);
    }

    #[test]
    fn language_names_are_stable() {
        assert_eq!(Language::Rust.name(), "rust");
        assert_eq!(Language::Unsupported.name(), "unsupported");
    }

    #[test]
    fn rust_is_supported_unsupported_is_not() {
        assert!(Language::Rust.is_supported());
        assert!(!Language::Unsupported.is_supported());
    }

    #[test]
    fn extension_is_case_sensitive() {
        // ".RS" is not matched; detection is conservative.
        assert_eq!(detect_language("X.RS"), Language::Unsupported);
    }

    #[test]
    fn nested_path_with_rs_extension_detected() {
        assert_eq!(detect_language("a/b/c/deep/file.rs"), Language::Rust);
    }

    #[test]
    fn language_is_copy_and_eq() {
        let a = Language::Rust;
        let b = a;
        assert_eq!(a, b);
    }
}
