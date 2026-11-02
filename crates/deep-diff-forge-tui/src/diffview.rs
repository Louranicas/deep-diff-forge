// DDF integration: R31, R32. Measure displayed cells and preserve grapheme boundaries after terminal-safety escaping; keep remaining chrome/wrap geometry visible as work.
// Design/status: docs/RECOMMENDATION_INTEGRATION.md#terminal-geometry
// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-tui--diffview
//! Diff rendering for both layouts, as one scrollable line list.
//!
//! The review is one continuous, scrollable document — every file stacked with a
//! section header, its anchored notes (as full-width rounded boxes that span both
//! columns), and each hunk's `@@` header and rows. [`diff_window`] builds only the
//! rows inside the viewport: it walks the document once to count rows and find each
//! file's section, then builds and paints only the visible window. Building and
//! painting every row on every frame cost 7.2 s per key at 900,000 lines, and passing
//! the offset through ratatui's `u16` paragraph scroll hid every row past 65,535
//! (bug hunt 2026-11-01). Inline shows a unified column; side-by-side composes
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
use deep_diff_forge_projection::{cell_width, clip_cells, expand_tabs, wrap_cells};
use deep_diff_forge_syntax::{Language, detect_language};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::ops::Range;

/// Context runs strictly longer than this collapse to one fold marker.
const FOLD_CONTEXT: usize = 6;

/// The rows of the review document inside a viewport, and where they sit in it.
pub(crate) struct DocumentWindow {
    /// The visible rows: at most the viewport's height.
    pub(crate) lines: Vec<Line<'static>>,
    /// Rows in the whole document.
    pub(crate) total: usize,
    /// The document row shown first (`lines[0]`).
    pub(crate) offset: usize,
}

/// The document rows from the selected file's section start plus `delta`, at most `height`
/// of them. The offset is clamped to the last row, so over-scrolling shows the document's end.
#[must_use]
pub(crate) fn diff_window(
    app: &ReviewApp,
    palette: &Palette,
    width: usize,
    delta: usize,
    height: usize,
) -> DocumentWindow {
    let mut count = Rows::new(0..0, false);
    let sections = document(&mut count, app, palette, width, None);
    let total = count.next;
    let base = sections
        .get(app.selected_index())
        .map_or(0, |section| section.start);
    let offset = base.saturating_add(delta).min(total.saturating_sub(1));
    let mut rows = Rows::new(offset..offset.saturating_add(height), true);
    document(&mut rows, app, palette, width, Some(&sections));
    DocumentWindow {
        lines: rows.lines,
        total,
        offset,
    }
}

/// Render the whole review as one document — every file stacked in ranked order — and return
/// the start line of each file's section. The selected file's header is emphasised; all other
/// files render the same way, so paging flows from one file straight into the next.
#[cfg(test)]
#[must_use]
pub(crate) fn diff_document(
    app: &ReviewApp,
    palette: &Palette,
    width: usize,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut rows = Rows::new(0..usize::MAX, false);
    let sections = document(&mut rows, app, palette, width, None);
    (rows.lines, sections.iter().map(|s| s.start).collect())
}

/// A sink for document rows: every row is counted, and only a row inside `window` is built.
/// With `stop` set, the walk ends once the window is full.
struct Rows {
    next: usize,
    window: Range<usize>,
    stop: bool,
    lines: Vec<Line<'static>>,
}

impl Rows {
    fn new(window: Range<usize>, stop: bool) -> Self {
        Self {
            next: 0,
            window,
            stop,
            lines: Vec::new(),
        }
    }

    fn push(&mut self, make: impl FnOnce() -> Line<'static>) {
        if self.window.contains(&self.next) {
            self.lines.push(make());
        }
        self.next += 1;
    }

    /// Whether any of the next `n` rows falls inside the window.
    fn wants(&self, n: usize) -> bool {
        self.next < self.window.end && self.next + n > self.window.start
    }

    fn extend(&mut self, lines: Vec<Line<'static>>) {
        for line in lines {
            self.push(|| line);
        }
    }

    /// Whether the walk can end: the window is full and the rows after it are not needed.
    fn done(&self) -> bool {
        self.stop && self.next >= self.window.end
    }
}

/// Walk the document into `rows` and return each file's section (rows from its header to its
/// last row). With `known` (the sections of an earlier walk at the same width and options), a
/// section that ends before the window is skipped without being walked.
fn document(
    rows: &mut Rows,
    app: &ReviewApp,
    palette: &Palette,
    width: usize,
    known: Option<&[Range<usize>]>,
) -> Vec<Range<usize>> {
    if app.is_empty() {
        rows.push(|| dim_line("no files in review", palette));
        return Vec::new();
    }
    let side = app.layout() == LayoutMode::SideBySide;
    let mut sections = Vec::with_capacity(app.file_count());
    for (i, file) in app.content().iter().enumerate() {
        if rows.done() {
            break;
        }
        if i > 0 {
            rows.push(Line::default);
        }
        let start = rows.next;
        if let Some(section) = known.and_then(|known| known.get(i))
            && section.end <= rows.window.start
        {
            rows.next = section.end;
        } else {
            file_section(
                rows,
                app,
                file,
                i == app.selected_index(),
                palette,
                side,
                width,
            );
        }
        sections.push(start..rows.next);
    }
    sections
}

fn file_section(
    rows: &mut Rows,
    app: &ReviewApp,
    file: &ReviewFile,
    selected: bool,
    palette: &Palette,
    side: bool,
    width: usize,
) {
    let lang = detect_language(&file.path);
    rows.push(|| section_header(file, selected, palette));
    rows.push(|| rule(palette, width));

    if app.show_notes() {
        for note in file_annotations(app.annotations(), &file.path) {
            rows.extend(note_box(note, palette, width));
        }
    }
    if file.patch_twin.hunks.is_empty() {
        rows.push(|| dim_line("(no textual diff)", palette));
        return;
    }
    for hunk in &file.patch_twin.hunks {
        if rows.done() {
            return;
        }
        if app.show_notes() {
            for note in hunk_annotations(app.annotations(), &file.path, hunk.id) {
                rows.extend(note_box(note, palette, width));
            }
        }
        rows.push(|| hunk_header(hunk, palette));
        push_rows(
            rows,
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
    rows: &mut Rows,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    options: RowOptions,
) {
    if options.side {
        push_side(rows, hunk, lang, palette, options.fold, options.width);
    } else {
        push_inline(rows, hunk, lang, palette, options);
    }
}

/// Inline (unified): one row per patch line; `-` and `+` are separate rows.
fn push_inline(
    rows: &mut Rows,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    options: RowOptions,
) {
    let mut ctx: Vec<&PatchLine> = Vec::new();
    for line in &hunk.lines {
        if rows.done() {
            return;
        }
        if line.kind == PatchLineKind::Context {
            ctx.push(line);
            continue;
        }
        flush_inline_ctx(rows, &mut ctx, lang, palette, options);
        inline_rows(rows, line, lang, palette, options);
    }
    flush_inline_ctx(rows, &mut ctx, lang, palette, options);
}

fn flush_inline_ctx(
    rows: &mut Rows,
    ctx: &mut Vec<&PatchLine>,
    lang: Language,
    palette: &Palette,
    options: RowOptions,
) {
    if options.fold && ctx.len() > FOLD_CONTEXT {
        let count = ctx.len();
        rows.push(|| fold_marker(count, palette));
    } else {
        for line in ctx.iter() {
            inline_rows(rows, line, lang, palette, options);
        }
    }
    ctx.clear();
}

/// Side-by-side: removed/added runs zipped into `old │ new` rows.
fn push_side(
    rows: &mut Rows,
    hunk: &PatchHunk,
    lang: Language,
    palette: &Palette,
    fold: bool,
    width: usize,
) {
    let col = width.saturating_sub(3) / 2;
    let mut ctx: Vec<(Option<&PatchLine>, Option<&PatchLine>)> = Vec::new();
    for pair in pair_hunk(&hunk.lines) {
        if rows.done() {
            return;
        }
        if is_context_pair(pair) {
            ctx.push(pair);
            continue;
        }
        flush_side_ctx(rows, &mut ctx, fold, lang, palette, col);
        rows.push(|| side_row(pair, lang, palette, col));
    }
    flush_side_ctx(rows, &mut ctx, fold, lang, palette, col);
}

fn flush_side_ctx<'a>(
    rows: &mut Rows,
    ctx: &mut Vec<(Option<&'a PatchLine>, Option<&'a PatchLine>)>,
    fold: bool,
    lang: Language,
    palette: &Palette,
    col: usize,
) {
    if fold && ctx.len() > FOLD_CONTEXT {
        let count = ctx.len();
        rows.push(|| fold_marker(count, palette));
    } else {
        for pair in ctx.iter() {
            rows.push(|| side_row(*pair, lang, palette, col));
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

/// One unwrapped inline row. Tabs are expanded before the row is measured and painted, as
/// the wrapped and side-by-side rows already were: ratatui drops a raw tab, so tab
/// indentation, and an indentation-only change, vanished from the default view (bug hunt
/// 2026-11-01).
fn inline_row(line: &PatchLine, lang: Language, palette: &Palette, width: usize) -> Line<'static> {
    let safe = display_safe(&line.text);
    inline_row_chunk(line, lang, palette, width, &expand_tabs(&safe), false)
}

fn inline_rows(
    rows: &mut Rows,
    line: &PatchLine,
    lang: Language,
    palette: &Palette,
    options: RowOptions,
) {
    let width = options.width;
    if !options.wrap_lines {
        rows.push(|| inline_row(line, lang, palette, width));
        return;
    }
    let text_width = inline_text_width(width);
    let safe = display_safe(&line.text);
    let count = chunk_count(&safe, text_width);
    if !rows.wants(count) {
        rows.next += count;
        return;
    }
    for (index, chunk) in chunks(&safe, text_width).into_iter().enumerate() {
        rows.push(|| inline_row_chunk(line, lang, palette, width, &chunk, index > 0));
    }
}

/// How many rows [`chunks`] splits already-safe text into. ASCII text is counted without
/// segmenting it: after `display_safe` every ASCII character but the tab is one cell wide, and
/// a tab is four.
fn chunk_count(safe: &str, width: usize) -> usize {
    if safe.is_ascii() {
        let tabs = safe.bytes().filter(|&b| b == b'\t').count();
        let cells = safe.len() + 3 * tabs;
        return cells.div_ceil(width.max(1)).max(1);
    }
    chunks(safe, width).len()
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
        let used = cell_width(text);
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
    wrap_cells(text, width)
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
/// is always rendered through [`display_safe`]. Every row is measured in terminal
/// cells, and the title shortens its anchor so the grounding label and the closing
/// corner always fit.
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
    let (prefix, label) = note_title(
        source_of(note).label(),
        &anchor,
        grounding_of(note).label(),
        w,
    );
    // Fixed cells besides the dashes: the space before them and the closing `╮`.
    let used = cell_width(&prefix) + cell_width(&label) + 2;
    let dashes = w.saturating_sub(used);
    let mut lines = vec![Line::from(vec![
        Span::styled(prefix, note_style),
        Span::styled(label, Style::default().fg(g_color)),
        Span::styled(format!(" {}╮", "─".repeat(dashes)), note_style),
    ])];
    for body in wrap(&display_safe(&note.body), inner) {
        let pad = inner.saturating_sub(cell_width(&body));
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

/// The note title's prefix and grounding label, fitted to a box `w` cells wide with room
/// for the space and corner after them. The anchor gives way first (clipped, with `…`),
/// then the source; the label is clipped only in a box too narrow for it. Counting chars
/// and never shortening the anchor let a long or wide file name push the label and the
/// corner off the pane (bug hunt 2026-11-01).
fn note_title(source: &str, anchor: &str, label: &str, w: usize) -> (String, String) {
    let tail = 2;
    let room = w.saturating_sub(cell_width(label) + tail);
    let head = format!("╭─ {source} note · ");
    let full = format!("{head}{anchor} · ");
    if cell_width(&full) <= room {
        return (full, label.to_string());
    }
    // The head, the separator after the anchor and the ellipsis.
    let fixed = cell_width(&head) + 3 + 1;
    if room > fixed {
        let clipped = clip_cells(anchor, room - fixed);
        return (format!("{head}{clipped}… · "), label.to_string());
    }
    let corner = "╭─ ";
    let label = clip_cells(label, w.saturating_sub(cell_width(corner) + tail));
    (corner.to_string(), label)
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
    deep_diff_forge_projection::fit_cells(safe, width)
}

/// Greedy word-wrap to `width` terminal cells, hard-splitting over-long words. Always
/// returns at least one (possibly empty) line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cell_width(&cur) + 1 + cell_width(word) <= width {
            cur.push(' ');
            cur.push_str(word);
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        // A word that fits is one piece; a longer one is split at cell boundaries.
        let mut pieces = wrap_cells(word, width);
        cur = pieces.pop().unwrap_or_default();
        lines.extend(pieces);
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
    use std::fmt::Write as _;

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

    /// A run of exactly `FOLD_CONTEXT` unchanged lines stays; one more folds to a single marker
    /// that counts it, in both layouts, and nothing folds with folding off.
    #[test]
    fn fold_threshold_is_exact_in_both_layouts() {
        for side in [false, true] {
            for (n, folded) in [(FOLD_CONTEXT, false), (FOLD_CONTEXT + 1, true)] {
                let mut src = format!("--- a/f.txt\n+++ b/f.txt\n@@ -1,{0} +1,{0} @@\n", n + 1);
                for i in 0..n {
                    let _ = writeln!(src, " ctx{i}");
                }
                src.push_str("-old\n+new\n");
                let mut a = ReviewApp::from_review(&parse(&src).unwrap());
                if side {
                    a.handle(AppEvent::ToggleLayout);
                }
                let out = render(&a, 120);
                let markers = out.matches("unchanged line").count();
                assert_eq!(markers, usize::from(folded), "side {side}, {n}: {out}");
                if folded {
                    assert!(
                        out.contains(&format!("··· {n} unchanged lines ···")),
                        "{out}"
                    );
                }
                assert_eq!(out.contains("ctx0"), !folded, "side {side}, {n}: {out}");
                a.handle(AppEvent::ToggleFold);
                let unfolded = render(&a, 120);
                assert!(!unfolded.contains("unchanged line"), "{unfolded}");
                assert!(unfolded.contains(&format!("ctx{}", n - 1)), "{unfolded}");
            }
        }
    }

    #[test]
    fn empty_review_is_a_placeholder() {
        let a = ReviewApp::new(Vec::new());
        assert!(render(&a, 80).contains("no files in review"));
        let (_, starts) = diff_document(&a, &ThemeKind::Dark.palette(), 80);
        assert_eq!(starts.len(), 0);
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
            .map(|l| l.spans.iter().map(|s| cell_width(&s.content)).sum())
            .collect();
        assert!(
            widths.iter().all(|&x| x == 70),
            "uneven box rows: {widths:?}"
        );
    }

    fn note_on(path: &str, body: &str, grounded: bool) -> AgentAnnotation {
        use deep_diff_forge_core::{AnnotationAnchor, AnnotationProvenance};
        AgentAnnotation {
            id: "n".to_string(),
            anchor: AnnotationAnchor::File {
                path: path.to_string(),
            },
            body: body.to_string(),
            provenance: AnnotationProvenance {
                agent: deep_diff_forge_agent::SYSTEM_AGENT_ID.to_string(),
                model: None,
                evidence: if grounded {
                    vec![format!("{path}:1")]
                } else {
                    Vec::new()
                },
            },
            grounded,
        }
    }

    fn row_cells(line: &Line<'static>) -> usize {
        line.spans.iter().map(|s| cell_width(&s.content)).sum()
    }

    /// Bug hunt 2026-11-01: the title was measured in chars and its anchor never shortened, so a
    /// CJK or long file name pushed the grounding label and the corner past the pane.
    #[test]
    fn note_title_keeps_its_label_and_corner_for_any_file_name() {
        let palette = ThemeKind::Dark.palette();
        let long = format!("{}.rs", "abcdefghijklmnopqrstuvwxyz".repeat(3));
        for path in [
            "src/中文中文中文中文中文中文中文.rs",
            long.as_str(),
            "lib.rs",
            "界",
        ] {
            for grounded in [true, false] {
                let note = note_on(path, "body 界界界 text", grounded);
                let label = grounding_of(&note).label();
                for width in [16, 30, 45, 64, 100] {
                    let lines = note_box(&note, &palette, width);
                    let w = width.max(16);
                    for line in lines.iter().filter(|l| !l.spans.is_empty()) {
                        assert_eq!(row_cells(line), w, "{path} at {width}: {line:?}");
                    }
                    let title = text(&lines[0]);
                    assert!(title.ends_with('╮'), "{title}");
                    if w >= 45 {
                        assert!(title.contains(&format!(" {label} ")), "{title}");
                    }
                }
            }
        }
        // A name that fits is shown whole, one that does not is clipped with an ellipsis.
        let fits = text(&note_box(&note_on("a/lib.rs", "b", true), &palette, 64)[0]);
        assert!(fits.starts_with("╭─ system note · lib.rs · "), "{fits}");
        let wide = note_on("中文中文中文中文中文中文中文.rs", "b", true);
        let clipped = text(&note_box(&wide, &palette, 64)[0]);
        assert!(clipped.starts_with("╭─ system note · 中文"), "{clipped}");
        assert!(clipped.contains("… · partially_grounded ─"), "{clipped}");
        // Too narrow for the source and anchor: the corner and the label remain.
        let narrow = text(&note_box(&wide, &palette, 30)[0]);
        assert_eq!(narrow, "╭─ partially_grounded ───────╮");
        let tiny = text(&note_box(&wide, &palette, 16)[0]);
        assert_eq!(tiny, "╭─ partially_g ╮");
    }

    #[test]
    fn note_title_clips_the_anchor_to_the_last_cell_that_fits() {
        // 45 cells: label 18 + 2 leaves 25; the head is 17, " · " 3 and "…" 1, so 4 anchor cells.
        let (prefix, label) = note_title("system", "abcdefgh.rs", "partially_grounded", 45);
        assert_eq!(prefix, "╭─ system note · abcd… · ");
        assert_eq!(label, "partially_grounded");
        // "abc.rs" needs 26 cells of prefix: whole at 36 (room 26), clipped at 35 (room 25).
        let (prefix, _) = note_title("system", "abc.rs", "grounded", 36);
        assert_eq!(prefix, "╭─ system note · abc.rs · ");
        let (prefix, _) = note_title("system", "abc.rs", "grounded", 35);
        assert_eq!(prefix, "╭─ system note · abc.… · ");
        // One anchor cell is the least worth showing; with none the source goes too.
        let (prefix, _) = note_title("system", "abcdef", "grounded", 32);
        assert_eq!(prefix, "╭─ system note · a… · ");
        let (prefix, _) = note_title("system", "abcdef", "grounded", 31);
        assert_eq!(prefix, "╭─ ");
    }

    #[test]
    fn note_body_wraps_and_pads_in_cells() {
        let palette = ThemeKind::Dark.palette();
        let body = format!("{} tail {}", "界".repeat(30), "word ".repeat(12));
        let lines = note_box(&note_on("x.rs", &body, true), &palette, 40);
        assert!(lines.len() > 4, "{lines:?}");
        for line in lines.iter().filter(|l| !l.spans.is_empty()) {
            assert_eq!(row_cells(line), 40, "{line:?}");
        }
        assert!(wrap(&"界".repeat(10), 7).iter().all(|l| cell_width(l) <= 7));
        assert_eq!(wrap("界界 ab", 5), vec!["界界", "ab"]);
        assert_eq!(wrap("界界 a", 6), vec!["界界 a"]);
        assert_eq!(wrap("ab 界界界", 5), vec!["ab", "界界", "界"]);
        assert_eq!(wrap("ab cd", 5), vec!["ab cd"]);
        assert_eq!(wrap("ab cde", 5), vec!["ab", "cde"]);
        assert_eq!(wrap("abcde fg", 5), vec!["abcde", "fg"]);
        assert_eq!(wrap("abcdefg h", 5), vec!["abcde", "fg h"]);
        assert_eq!(wrap("  ", 5), vec![""]);
    }

    /// Bug hunt 2026-11-01: the default (unwrapped) inline row handed raw tabs to ratatui,
    /// which drops them, so tab indentation and a dedent were invisible.
    #[test]
    fn inline_rows_show_tab_indentation_in_every_layout() {
        let src = "--- a/x.py\n+++ b/x.py\n@@ -1,3 +1,3 @@\n def f():\n-\tif a:\n-\t\treturn 1\n+\tif a:\n+\treturn 1\n";
        let files = parse(src).unwrap();
        let mut a = ReviewApp::from_review(&files);
        a.handle(AppEvent::ToggleNotes);
        for step in [
            None,
            Some(AppEvent::ToggleWrap),
            Some(AppEvent::ToggleLayout),
        ] {
            if let Some(event) = step {
                a.handle(event);
            }
            let out = render(&a, 100);
            assert!(!out.contains('\t'), "{out}");
            assert!(out.contains("-         return 1"), "{out}");
            assert!(out.contains("+     return 1"), "{out}");
        }
        // The tint behind a changed row covers its expanded text exactly.
        let palette = ThemeKind::Dark.palette();
        let line = &files[0].patch_twin.hunks[0].lines[2];
        let row = inline_row(line, Language::Unsupported, &palette, 60);
        assert_eq!(row_cells(&row), 60, "{row:?}");
    }

    fn big_review(n: usize) -> ReviewApp {
        let mut src = format!("--- /dev/null\n+++ b/a_big.txt\n@@ -0,0 +1,{n} @@\n");
        for i in 0..n {
            let _ = writeln!(src, "+bigline {i}");
        }
        src.push_str("--- a/m.txt\n+++ b/m.txt\n@@ -1,1 +1,1 @@\n-mid_old\n+mid_new\n");
        src.push_str("--- a/z.txt\n+++ b/z.txt\n@@ -1,1 +1,1 @@\n-z_old\n+z_new\n");
        ReviewApp::from_review(&parse(&src).unwrap())
    }

    /// The window is exactly the rows of the whole document from its offset, for every file,
    /// delta, layout and wrap setting.
    #[test]
    fn diff_window_matches_the_whole_document() {
        let palette = ThemeKind::Dark.palette();
        let mut a = big_review(40);
        for step in [
            None,
            Some(AppEvent::ToggleWrap),
            Some(AppEvent::ToggleLayout),
        ] {
            if let Some(event) = step {
                a.handle(event);
            }
            a.handle(AppEvent::Top);
            for file in 0..a.file_count() {
                let (whole, starts) = diff_document(&a, &palette, 50);
                for delta in [0, 1, 7, 30, 1000] {
                    for height in [1, 5, 20] {
                        let w = diff_window(&a, &palette, 50, delta, height);
                        assert_eq!(w.total, whole.len());
                        let offset = (starts[file] + delta).min(whole.len() - 1);
                        assert_eq!(w.offset, offset);
                        let want: Vec<String> =
                            whole.iter().skip(offset).take(height).map(text).collect();
                        let got: Vec<String> = w.lines.iter().map(text).collect();
                        assert_eq!(got, want, "file {file} delta {delta} height {height}");
                    }
                }
                a.handle(AppEvent::Next);
            }
        }
        let empty = diff_window(&ReviewApp::new(Vec::new()), &palette, 50, 3, 10);
        assert_eq!((empty.total, empty.offset), (1, 0));
        assert_eq!(text(&empty.lines[0]), "no files in review");
    }

    /// `wants` decides whether a wrapped line is segmented at all, so a wrong answer costs only
    /// time; its contract is checked at each edge of the window.
    #[test]
    fn rows_want_exactly_the_runs_that_meet_the_window() {
        let mut rows = Rows::new(10..20, true);
        assert!(!rows.wants(5));
        assert!(!rows.wants(10));
        assert!(rows.wants(11));
        rows.next = 19;
        assert!(rows.wants(1));
        rows.next = 20;
        assert!(!rows.wants(1));
        assert!(!rows.wants(100));
    }

    #[test]
    fn chunk_count_is_the_number_of_chunks() {
        let mut texts: Vec<String> = (0..60).map(|n| "x".repeat(n)).collect();
        texts.extend([
            "\t".into(),
            "a\tb\t\tc".into(),
            "界界界a".into(),
            "e\u{301}e".into(),
        ]);
        for text in &texts {
            for width in 0..25 {
                assert_eq!(
                    chunk_count(text, width),
                    chunks(text, width).len(),
                    "{text:?} at {width}"
                );
            }
        }
    }

    /// Wrapped rows are counted, and built only when visible, in every window position.
    #[test]
    fn wrapped_windows_match_the_whole_document() {
        let mut src = String::from("--- /dev/null\n+++ b/w.rs\n@@ -0,0 +1,8 @@\n");
        for i in 0..8 {
            let _ = writeln!(src, "+{}", "abcdefghij".repeat(i));
        }
        let mut a = ReviewApp::from_review(&parse(&src).unwrap());
        a.handle(AppEvent::ToggleWrap);
        let palette = ThemeKind::Dark.palette();
        let (whole, _) = diff_document(&a, &palette, 30);
        for delta in 0..whole.len() {
            for height in [1, 2, 3] {
                let w = diff_window(&a, &palette, 30, delta, height);
                let want: Vec<String> = whole.iter().skip(delta).take(height).map(text).collect();
                assert_eq!(
                    w.lines.iter().map(text).collect::<Vec<_>>(),
                    want,
                    "{delta}"
                );
            }
        }
    }

    /// Only the viewport is built, and the walk stops once it is full: a frame no longer
    /// costs the whole review (bug hunt 2026-11-01: 7.2 s per key at 900,000 lines).
    #[test]
    fn a_window_walk_stops_after_the_viewport() {
        let palette = ThemeKind::Dark.palette();
        let a = big_review(5_000);
        let mut count = Rows::new(0..0, false);
        let sections = document(&mut count, &a, &palette, 80, None);
        assert_eq!(count.lines.len(), 0);
        assert!(count.next > 5_000);
        assert_eq!(sections.len(), 3);
        let mut rows = Rows::new(10..30, true);
        document(&mut rows, &a, &palette, 80, Some(&sections));
        assert_eq!(rows.lines.len(), 20);
        assert!(rows.next <= 31, "walked {} rows", rows.next);
        // A section before the window is skipped by its known extent, not walked.
        let mid = sections[1].start;
        let mut rows = Rows::new(mid..mid + 3, true);
        let walked = document(&mut rows, &a, &palette, 80, Some(&sections));
        assert_eq!(walked[0], sections[0]);
        assert!(text(&rows.lines[0]).contains("m.txt"), "{:?}", rows.lines);
        let mut skipped = Rows::new(sections[0].end..sections[0].end + 1, true);
        let skipped_sections = document(&mut skipped, &a, &palette, 80, Some(&sections));
        assert_eq!(skipped_sections[0], sections[0]);
        assert_eq!(text(&skipped.lines[0]), "");
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
