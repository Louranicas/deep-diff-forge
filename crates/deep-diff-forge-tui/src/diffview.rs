//! Diff rendering for both layouts, as one scrollable line list.
//!
//! [`diff_document`] renders the whole review as one continuous, scrollable
//! document — every file stacked with a section header, its anchored notes (as
//! full-width rounded boxes that span both columns), and each hunk's `@@` header
//! and rows — returning per-file start lines so the caller can scroll to the
//! selection. Inline shows a unified column; side-by-side composes
//! `old │ new` rows with a coloured change-bar gutter. Both fold long unchanged
//! runs to a `··· N unchanged lines ···` marker. Rows are syntax-coloured via
//! [`crate::paint`] (terminal-safe), tinted by change kind.

use crate::notes::{file_annotations, hunk_annotations};
use crate::paint::{safe_span, themed_spans};
use crate::state::{LayoutMode, ReviewApp};
use crate::theme::Palette;
use deep_diff_forge_agent::{GroundingLevel, anchor_path, grounding_of, source_of};
use deep_diff_forge_core::{
    AgentAnnotation, PatchHunk, PatchLine, PatchLineKind, ReviewFile, display_safe,
};
use deep_diff_forge_graph::change_counts;
use deep_diff_forge_syntax::{Language, detect_language};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Context runs strictly longer than this collapse to one fold marker.
const FOLD_CONTEXT: usize = 6;

/// Render the whole review as one continuous, scrollable diff document — every
/// file stacked in ranked order — and return the start line of each file's
/// section so the caller can scroll to the selection. The selected file's header
/// is emphasised; all other files render the same way, so paging flows from one
/// file straight into the next.
#[must_use]
pub(crate) fn diff_document(
    app: &ReviewApp,
    palette: &Palette,
    width: usize,
) -> (Vec<Line<'static>>, Vec<usize>) {
    if app.is_empty() {
        return (vec![dim_line("no files in review", palette)], Vec::new());
    }
    let side = app.layout() == LayoutMode::SideBySide;
    let mut lines = Vec::new();
    let mut starts = Vec::with_capacity(app.file_count());
    for (i, file) in app.content().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        starts.push(lines.len());
        file_section(
            &mut lines,
            app,
            file,
            i == app.selected_index(),
            palette,
            side,
            width,
        );
    }
    (lines, starts)
}

fn file_section(
    lines: &mut Vec<Line<'static>>,
    app: &ReviewApp,
    file: &ReviewFile,
    selected: bool,
    palette: &Palette,
    side: bool,
    width: usize,
) {
    let lang = detect_language(&file.path);
    lines.push(section_header(file, selected, palette));
    lines.push(rule(palette, width));

    if app.show_notes() {
        for note in file_annotations(app.annotations(), &file.path) {
            lines.extend(note_box(note, palette, width));
        }
    }
    if file.patch_twin.hunks.is_empty() {
        lines.push(dim_line("(no textual diff)", palette));
        return;
    }
    for hunk in &file.patch_twin.hunks {
        if app.show_notes() {
            for note in hunk_annotations(app.annotations(), &file.path, hunk.id) {
                lines.extend(note_box(note, palette, width));
            }
        }
        lines.push(hunk_header(hunk, palette));
        push_rows(
            lines,
            hunk,
            lang,
            palette,
            RowOptions {
                fold: app.fold(),
                side,
                width,
                wrap_lines: app.wrap_lines(),
            },
        );
    }
}

// ===== row emission with context folding =====

#[derive(Clone, Copy)]
struct RowOptions {
    fold: bool,
    side: bool,
    width: usize,
    wrap_lines: bool,
}

fn push_rows(
    lines: &mut Vec<Line<'static>>,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    options: RowOptions,
) {
    if options.side {
        push_side(lines, hunk, lang, palette, options.fold, options.width);
    } else {
        push_inline(
            lines,
            hunk,
            lang,
            palette,
            options.fold,
            options.width,
            options.wrap_lines,
        );
    }
}

/// Inline (unified): one row per patch line; `-` and `+` are separate rows.
fn push_inline(
    lines: &mut Vec<Line<'static>>,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    fold: bool,
    width: usize,
    wrap_lines: bool,
) {
    let mut ctx: Vec<&PatchLine> = Vec::new();
    for line in &hunk.lines {
        if line.kind == PatchLineKind::Context {
            ctx.push(line);
            continue;
        }
        flush_inline_ctx(lines, &mut ctx, fold, lang, palette, width, wrap_lines);
        lines.extend(inline_rows(line, lang, palette, width, wrap_lines));
    }
    flush_inline_ctx(lines, &mut ctx, fold, lang, palette, width, wrap_lines);
}

fn flush_inline_ctx(
    lines: &mut Vec<Line<'static>>,
    ctx: &mut Vec<&PatchLine>,
    fold: bool,
    lang: Language,
    palette: &Palette,
    width: usize,
    wrap_lines: bool,
) {
    if fold && ctx.len() > FOLD_CONTEXT {
        lines.push(fold_marker(ctx.len(), palette));
    } else {
        for line in ctx.iter() {
            lines.extend(inline_rows(line, lang, palette, width, wrap_lines));
        }
    }
    ctx.clear();
}

/// Side-by-side: removed/added runs zipped into `old │ new` rows.
fn push_side(
    lines: &mut Vec<Line<'static>>,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    fold: bool,
    width: usize,
) {
    let col = width.saturating_sub(3) / 2;
    let mut ctx: Vec<(Option<&PatchLine>, Option<&PatchLine>)> = Vec::new();
    for pair in pair_hunk(&hunk.lines) {
        if is_context_pair(pair) {
            ctx.push(pair);
            continue;
        }
        flush_side_ctx(lines, &mut ctx, fold, lang, palette, col);
        lines.push(side_row(pair, lang, palette, col));
    }
    flush_side_ctx(lines, &mut ctx, fold, lang, palette, col);
}

fn flush_side_ctx<'a>(
    lines: &mut Vec<Line<'static>>,
    ctx: &mut Vec<(Option<&'a PatchLine>, Option<&'a PatchLine>)>,
    fold: bool,
    lang: Language,
    palette: &Palette,
    col: usize,
) {
    if fold && ctx.len() > FOLD_CONTEXT {
        lines.push(fold_marker(ctx.len(), palette));
    } else {
        for pair in ctx.iter() {
            lines.push(side_row(*pair, lang, palette, col));
        }
    }
    ctx.clear();
}

fn side_row(
    pair: (Option<&PatchLine>, Option<&PatchLine>),
    lang: Language,
    palette: &Palette,
    col: usize,
) -> Line<'static> {
    let mut spans = side_cell(pair.0, false, lang, palette, col);
    spans.push(Span::styled(" │ ", Style::default().fg(palette.border)));
    spans.extend(side_cell(pair.1, true, lang, palette, col));
    Line::from(spans)
}

// ===== inline cell =====

fn inline_row(line: &PatchLine, lang: Language, palette: &Palette, width: usize) -> Line<'static> {
    inline_row_chunk(line, lang, palette, width, &display_safe(&line.text), false)
}

fn inline_rows(
    line: &PatchLine,
    lang: Language,
    palette: &Palette,
    width: usize,
    wrap_lines: bool,
) -> Vec<Line<'static>> {
    if !wrap_lines {
        return vec![inline_row(line, lang, palette, width)];
    }
    let text_width = inline_text_width(width);
    chunks(&display_safe(&line.text), text_width)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| inline_row_chunk(line, lang, palette, width, &chunk, index > 0))
        .collect()
}

fn inline_row_chunk(
    line: &PatchLine,
    lang: Language,
    palette: &Palette,
    width: usize,
    text: &str,
    continuation: bool,
) -> Line<'static> {
    let (marker, fg, bg) = kind_style(line.kind, palette);
    let base = bg.map_or_else(Style::default, |b| Style::default().bg(b));
    let (old, new) = if continuation {
        (String::new(), String::new())
    } else {
        (
            line.old_line.map(|n| n.to_string()).unwrap_or_default(),
            line.new_line.map(|n| n.to_string()).unwrap_or_default(),
        )
    };
    let marker = if continuation { '↳' } else { marker };
    let mut spans = vec![
        bar_span(line.kind, palette),
        Span::styled(format!("{old:>5} "), Style::default().fg(palette.dim)),
        Span::styled(format!("{new:>5} "), Style::default().fg(palette.dim)),
        Span::styled(format!("{marker} "), base.fg(fg)),
    ];
    spans.extend(themed_spans(lang, text, palette, base));
    if bg.is_some() {
        let text_width = inline_text_width(width);
        let used = text.chars().count();
        if used < text_width {
            spans.push(Span::styled(" ".repeat(text_width - used), base));
        }
    }
    Line::from(spans)
}

#[must_use]
fn inline_text_width(width: usize) -> usize {
    width.saturating_sub(15).max(1)
}

/// Hard-split already-safe text into fixed-width character chunks.
#[must_use]
fn chunks(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() == width {
            out.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

// ===== side-by-side cell =====

fn side_cell(
    cell: Option<&PatchLine>,
    right_side: bool,
    lang: Language,
    palette: &Palette,
    col: usize,
) -> Vec<Span<'static>> {
    let Some(line) = cell else {
        return vec![Span::raw(" ".repeat(col))];
    };
    let (marker, fg, bg) = kind_style(line.kind, palette);
    let base = bg.map_or_else(Style::default, |b| Style::default().bg(b));
    let number = if right_side {
        line.new_line
    } else {
        line.old_line
    };
    let numstr = number.map(|n| n.to_string()).unwrap_or_default();
    let text_width = col.saturating_sub(8).max(1);
    let shown = truncate_pad(&display_safe(&line.text), text_width);
    let mut spans = vec![
        bar_span(line.kind, palette),
        Span::styled(format!("{numstr:>4} "), Style::default().fg(palette.dim)),
        Span::styled(format!("{marker} "), base.fg(fg)),
    ];
    spans.extend(themed_spans(lang, &shown, palette, base));
    spans
}

/// The leading change-bar gutter glyph (`▌` in the change colour, space for
/// context) — the coloured rail down the edge of changed rows.
fn bar_span(kind: PatchLineKind, palette: &Palette) -> Span<'static> {
    match kind {
        PatchLineKind::Added => Span::styled("▌", Style::default().fg(palette.added)),
        PatchLineKind::Removed => Span::styled("▌", Style::default().fg(palette.removed)),
        PatchLineKind::Context => Span::raw(" "),
    }
}

// ===== pairing =====

fn pair_hunk(lines: &[PatchLine]) -> Vec<(Option<&PatchLine>, Option<&PatchLine>)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == PatchLineKind::Context {
            rows.push((Some(&lines[i]), Some(&lines[i])));
            i += 1;
            continue;
        }
        let mut removed = Vec::new();
        while i < lines.len() && lines[i].kind == PatchLineKind::Removed {
            removed.push(&lines[i]);
            i += 1;
        }
        let mut added = Vec::new();
        while i < lines.len() && lines[i].kind == PatchLineKind::Added {
            added.push(&lines[i]);
            i += 1;
        }
        for k in 0..removed.len().max(added.len()) {
            rows.push((removed.get(k).copied(), added.get(k).copied()));
        }
    }
    rows
}

fn is_context_pair(pair: (Option<&PatchLine>, Option<&PatchLine>)) -> bool {
    matches!(pair.0, Some(l) if l.kind == PatchLineKind::Context)
}

// ===== headers, notes, shared bits =====

/// A file-section header: `▌ path [status] +A -B`. The selected file's row is
/// accented and bar-marked so it stands out in the continuous document.
fn section_header(file: &ReviewFile, selected: bool, palette: &Palette) -> Line<'static> {
    let (add, del) = change_counts(file);
    let (marker, path_style) = if selected {
        (
            "▌ ",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "  ",
            Style::default().fg(palette.fg).add_modifier(Modifier::BOLD),
        )
    };
    Line::from(vec![
        Span::styled(marker, Style::default().fg(palette.accent)),
        safe_span(&file.path, path_style),
        Span::styled(
            format!("  [{}]", file.status.label()),
            Style::default().fg(palette.dim),
        ),
        Span::styled(format!("  +{add}"), Style::default().fg(palette.added)),
        Span::styled(format!(" -{del}"), Style::default().fg(palette.removed)),
    ])
}

/// A full-width horizontal rule under a section header.
fn rule(palette: &Palette, width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(1)),
        Style::default().fg(palette.border),
    ))
}

fn hunk_header(hunk: &PatchHunk, palette: &Palette) -> Line<'static> {
    let old = hunk.old_start.unwrap_or(0);
    let new = hunk.new_start.unwrap_or(0);
    Line::from(Span::styled(
        format!("@@ -{old} +{new} @@"),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::DIM),
    ))
}

fn fold_marker(count: usize, palette: &Palette) -> Line<'static> {
    let plural = if count == 1 { "" } else { "s" };
    Line::from(Span::styled(
        format!("    ··· {count} unchanged line{plural} ···"),
        Style::default().fg(palette.dim),
    ))
}

fn dim_line(text: &'static str, palette: &Palette) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(palette.dim)))
}

fn base_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A full-width bordered note box with a grounding-coloured title that names the
/// note's source and anchor (e.g. `system note · lib.rs · grounded`). The body
/// is always rendered through [`display_safe`].
fn note_box(note: &AgentAnnotation, palette: &Palette, width: usize) -> Vec<Line<'static>> {
    let w = width.max(16);
    let inner = w.saturating_sub(4);
    let note_style = Style::default().fg(palette.note);
    let g_color = if grounding_of(note) == GroundingLevel::Grounded {
        palette.grounded
    } else {
        palette.ungrounded
    };

    let anchor = display_safe(base_of(anchor_path(&note.anchor)));
    let prefix = format!("╭─ {} note · {anchor} · ", source_of(note).label());
    let label = grounding_of(note).label();
    // Fixed chars besides the dashes: the space before them and the closing `╮`.
    let used = prefix.chars().count() + label.chars().count() + 2;
    let dashes = w.saturating_sub(used);
    let mut lines = vec![Line::from(vec![
        Span::styled(prefix, note_style),
        Span::styled(label.to_string(), Style::default().fg(g_color)),
        Span::styled(format!(" {}╮", "─".repeat(dashes)), note_style),
    ])];
    for body in wrap(&display_safe(&note.body), inner) {
        let pad = inner.saturating_sub(body.chars().count());
        lines.push(Line::from(vec![
            Span::styled("│ ", note_style),
            Span::styled(body, Style::default().fg(palette.fg)),
            Span::styled(format!("{} │", " ".repeat(pad)), note_style),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(w.saturating_sub(2))),
        note_style,
    )));
    lines.push(Line::default());
    lines
}

fn kind_style(kind: PatchLineKind, palette: &Palette) -> (char, Color, Option<Color>) {
    match kind {
        PatchLineKind::Added => ('+', palette.added, Some(palette.added_bg)),
        PatchLineKind::Removed => ('-', palette.removed, Some(palette.removed_bg)),
        PatchLineKind::Context => (' ', palette.fg, None),
    }
}

/// Truncate `safe` (already terminal-safe) to `width` chars, padding with
/// spaces so every cell is exactly `width` columns wide.
fn truncate_pad(safe: &str, width: usize) -> String {
    let count = safe.chars().count();
    if count >= width {
        safe.chars().take(width).collect()
    } else {
        format!("{safe}{}", " ".repeat(width - count))
    }
}

/// Greedy word-wrap to `width` columns, hard-splitting over-long words. Always
/// returns at least one (possibly empty) line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if word.chars().count() > width {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(std::mem::take(&mut chunk));
                }
                chunk.push(ch);
            }
            cur = chunk;
            continue;
        }
        let extra = if cur.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if cur.chars().count() + extra > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppEvent, ReviewApp};
    use crate::theme::ThemeKind;
    use deep_diff_forge_patch::parse;

    const DIFF: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 fn keep() {}
 a
 b
-let old = 1;
+let renamed = 2;
 c
";

    fn app() -> ReviewApp {
        let files = parse(DIFF).unwrap();
        let notes = crate::notes::engine_annotations(&files, &deep_diff_forge_graph::rank(&files));
        ReviewApp::from_review_with_annotations(&files, notes)
    }

    fn text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn render(app: &ReviewApp, width: usize) -> String {
        diff_document(app, &ThemeKind::Dark.palette(), width)
            .0
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn header_shows_path_status_and_counts() {
        let out = render(&app(), 100);
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("[modified]"));
        assert!(out.contains("+1") && out.contains("-1"));
    }

    #[test]
    fn inline_shows_change_content_and_hunk_header() {
        let out = render(&app(), 100);
        assert!(out.contains("let renamed = 2;"));
        assert!(out.contains("let old = 1;"));
        assert!(out.contains("@@ -1 +1 @@"));
    }

    #[test]
    fn note_box_titles_source_anchor_and_grounding() {
        let out = render(&app(), 100);
        assert!(out.contains("system note"));
        assert!(out.contains("lib.rs")); // anchor basename in the title
        assert!(out.contains("grounded"));
        assert!(out.contains('╭') && out.contains('╰'));
    }

    #[test]
    fn notes_hidden_when_toggled_off() {
        let mut a = app();
        a.handle(AppEvent::ToggleNotes);
        assert!(!render(&a, 100).contains("system note"));
    }

    #[test]
    fn side_by_side_has_divider_and_both_sides() {
        let mut a = app();
        a.handle(AppEvent::ToggleLayout);
        let out = render(&a, 120);
        assert!(out.contains('│'));
        assert!(out.contains("let old = 1;"));
        assert!(out.contains("let renamed = 2;"));
    }

    #[test]
    fn change_rows_carry_a_gutter_bar() {
        let out = render(&app(), 100);
        assert!(out.contains('▌'), "changed rows should show the colour bar");
    }

    #[test]
    fn folding_collapses_long_context() {
        let src = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,9 +1,9 @@\n a\n b\n c\n d\n e\n f\n g\n h\n-i\n+I\n";
        let files = parse(src).unwrap();
        let mut a = ReviewApp::from_review(&files);
        assert!(render(&a, 100).contains("unchanged line"));
        a.handle(AppEvent::ToggleFold);
        assert!(!render(&a, 100).contains("unchanged line"));
    }

    #[test]
    fn folding_works_in_side_by_side_too() {
        let src = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,9 +1,9 @@\n a\n b\n c\n d\n e\n f\n g\n h\n-i\n+I\n";
        let files = parse(src).unwrap();
        let mut a = ReviewApp::from_review(&files);
        a.handle(AppEvent::ToggleLayout);
        assert!(render(&a, 120).contains("unchanged line"));
    }

    #[test]
    fn empty_review_is_a_placeholder() {
        let a = ReviewApp::new(Vec::new());
        assert!(render(&a, 80).contains("no files in review"));
        let (_, starts) = diff_document(&a, &ThemeKind::Dark.palette(), 80);
        assert!(starts.is_empty());
    }

    #[test]
    fn document_stacks_all_files_with_section_starts() {
        let multi = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-c
+d
";
        let files = parse(multi).unwrap();
        let a = ReviewApp::from_review(&files);
        let (lines, starts) = diff_document(&a, &ThemeKind::Dark.palette(), 100);
        // Both files appear in one document, with a recorded start per file.
        assert_eq!(starts.len(), 2);
        let out: String = lines.iter().map(text).collect::<Vec<_>>().join("\n");
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("src/other.rs"));
        // The second file's section starts after the first file's lines.
        assert!(starts[1] > starts[0]);
    }

    #[test]
    fn selected_file_section_is_bar_marked() {
        let multi = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,1 +1,1 @@
-c
+d
";
        let files = parse(multi).unwrap();
        let mut a = ReviewApp::from_review(&files);
        let (lines, starts) = diff_document(&a, &ThemeKind::Dark.palette(), 100);
        // The first (selected) file's header row carries the ▌ bar.
        assert!(text(&lines[starts[0]]).starts_with('▌'));
        // After moving selection, the second file's header is bar-marked instead.
        a.handle(AppEvent::Next);
        let (lines2, starts2) = diff_document(&a, &ThemeKind::Dark.palette(), 100);
        assert!(text(&lines2[starts2[1]]).starts_with('▌'));
        assert!(!text(&lines2[starts2[0]]).starts_with('▌'));
    }

    #[test]
    fn binary_file_reports_no_textual_diff() {
        let files =
            parse("diff --git a/x.bin b/x.bin\nBinary files a/x.bin and b/x.bin differ\n").unwrap();
        let a = ReviewApp::from_review(&files);
        assert!(render(&a, 80).contains("(no textual diff)"));
    }

    #[test]
    fn malicious_content_is_neutralised_in_both_layouts() {
        let evil = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-a\n+\u{1b}[2Jb\n";
        let files = parse(evil).unwrap();
        let mut a = ReviewApp::from_review(&files);
        assert!(!render(&a, 100).contains('\u{1b}'));
        a.handle(AppEvent::ToggleLayout);
        assert!(!render(&a, 100).contains('\u{1b}'));
    }

    #[test]
    fn wrap_toggle_wraps_long_inline_rows() {
        let long = "0123456789abcdefghijklmnopqrstuvwxyz";
        let src = format!("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-old\n+{long}\n");
        let files = parse(&src).unwrap();
        let mut a = ReviewApp::from_review(&files);
        assert!(!render(&a, 24).contains('↳'));
        a.handle(AppEvent::ToggleWrap);
        let out = render(&a, 24);
        assert!(out.contains('↳'));
        assert!(out.contains("012345678"));
        assert!(out.contains("9abcdefgh"));
    }

    #[test]
    fn wrapped_unsafe_content_stays_escaped() {
        let evil = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-old\n+\u{1b}[2Jabcdefghijklmnopqrstuvwxyz\n";
        let files = parse(evil).unwrap();
        let mut a = ReviewApp::from_review(&files);
        a.handle(AppEvent::ToggleWrap);
        let out = render(&a, 24);
        assert!(!out.contains('\u{1b}'));
        assert!(out.contains("\\x1b"));
        assert!(out.contains('↳'));
    }

    #[test]
    fn note_box_borders_are_flush_and_equal_width() {
        let notes = crate::notes::engine_annotations(
            &parse(DIFF).unwrap(),
            &deep_diff_forge_graph::rank(&parse(DIFF).unwrap()),
        );
        let lines = note_box(&notes[0], &ThemeKind::Dark.palette(), 70);
        let widths: Vec<usize> = lines
            .iter()
            .filter(|l| !l.spans.is_empty())
            .map(|l| l.spans.iter().map(|s| s.content.chars().count()).sum())
            .collect();
        assert!(
            widths.iter().all(|&x| x == 70),
            "uneven box rows: {widths:?}"
        );
    }

    #[test]
    fn wrap_and_truncate_behave() {
        assert!(
            wrap("alpha beta gamma", 5)
                .iter()
                .all(|l| l.chars().count() <= 5)
        );
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(chunks("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(truncate_pad("hi", 5).chars().count(), 5);
        assert_eq!(truncate_pad("hello world", 5), "hello");
    }

    #[test]
    fn pair_hunk_zips_change_block() {
        let files = parse(DIFF).unwrap();
        let pairs = pair_hunk(&files[0].patch_twin.hunks[0].lines);
        assert!(pairs.iter().any(|(l, r)| {
            l.is_some_and(|x| x.kind == PatchLineKind::Removed)
                && r.is_some_and(|y| y.kind == PatchLineKind::Added)
        }));
    }

    #[test]
    fn base_of_strips_dirs() {
        assert_eq!(base_of("a/b/c.rs"), "c.rs");
        assert_eq!(base_of("c.rs"), "c.rs");
    }

    #[test]
    fn note_box_title_escapes_malicious_anchor_path() {
        // Regression: FINDING-001 — the anchor (file path) in the note-box title
        // must pass through display_safe before reaching the Span, otherwise an
        // attacker-controlled filename containing ESC/CSI/OSC bytes reaches the
        // terminal and can poison the reviewer's scrollback or hijack the clipboard.
        use deep_diff_forge_core::{AnnotationAnchor, AnnotationProvenance};
        let evil_path = "src/\x1b[2J\x1b]52;;evilpayload\x07evil.rs".to_string();
        let note = AgentAnnotation {
            id: "test-id".to_string(),
            anchor: AnnotationAnchor::File {
                path: evil_path.clone(),
            },
            body: "safe body".to_string(),
            provenance: AnnotationProvenance {
                agent: "system".to_string(),
                model: None,
                evidence: vec!["src/evil.rs:1".to_string()],
            },
            grounded: true,
        };
        let palette = ThemeKind::Dark.palette();
        let lines = note_box(&note, &palette, 80);
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            !all_text.contains('\x1b'),
            "raw ESC escaped into note-box title: {all_text:?}"
        );
        assert!(
            !all_text.contains('\x07'),
            "raw BEL (OSC terminator) in note-box title: {all_text:?}"
        );
        // The sanitised representation of ESC should appear instead.
        assert!(
            all_text.contains("\\x1b") || all_text.contains("\\u{1b}"),
            "no visible escape representation found in sanitised title"
        );
    }

    #[test]
    fn kind_style_distinguishes_changes() {
        let p = ThemeKind::Dark.palette();
        assert_eq!(kind_style(PatchLineKind::Added, &p).0, '+');
        assert_eq!(kind_style(PatchLineKind::Removed, &p).0, '-');
        assert_eq!(kind_style(PatchLineKind::Context, &p).0, ' ');
        assert!(kind_style(PatchLineKind::Context, &p).2.is_none());
    }
}
