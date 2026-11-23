//! `deep-diff-forge.diagnostics.v0` in Rust (F5b): the typed model of the document
//! (`docs/reference/diagnostics.md`), its reader, which follows the schema's stability rule (a
//! member the model does not know is ignored), and its writer of the documented byte form, with
//! which [`Client::diagnostics`](crate::Client::diagnostics) rebuilds the document a daemon sent
//! in pages and checks it against `page.document_sha256`.

use crate::Error;
use serde_json::Value;
use std::collections::BTreeMap;

/// The schema this model reads.
pub const DIAGNOSTICS_SCHEMA: &str = "deep-diff-forge.diagnostics.v0";

/// A `deep-diff-forge.diagnostics.v0` document: every label the ranking raises, located. Its
/// members are the schema's; [`Diagnostics::to_json`] is the document's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostics {
    /// SHA-256 of the exact input bytes, as in `rank.v0`.
    pub input_sha256: String,
    /// The engine that wrote the document.
    pub tool: DiagnosticsTool,
    /// Which ranker judges a stop, when the writer knows it.
    pub judge: DiagnosticsJudge,
    /// The document's counts.
    pub counts: DiagnosticsCounts,
    /// The files whose located diagnostics were cut at 32, in file order.
    pub cut: Vec<CutFile>,
    /// The diagnostics, in the document's order.
    pub diagnostics: Vec<Diagnostic>,
    /// The sentence of every repair or substitute id the diagnostics name.
    pub texts: BTreeMap<String, String>,
    bytes: String,
}

/// `tool`: `{name, version, scorer_sha256}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiagnosticsTool {
    /// `deep-diff-forge`.
    pub name: String,
    /// The engine's version.
    pub version: String,
    /// The SHA-256 of the engine's scorer.
    pub scorer_sha256: String,
}

/// `judge`: `{state, ranker_sha256}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiagnosticsJudge {
    /// `unknown`, `same`, `different` or `none_admitted`.
    pub state: String,
    /// The SHA-256 of the ranker the policy admits, when one is.
    pub ranker_sha256: Option<String>,
}

/// `counts`: `{files, diagnostics, agent, person, cut_files}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiagnosticsCounts {
    /// Files ranked.
    pub files: u64,
    /// Diagnostics in the document.
    pub diagnostics: u64,
    /// Diagnostics whose audience is `agent`.
    pub agent: u64,
    /// Diagnostics whose audience is `person`.
    pub person: u64,
    /// Files listed in `cut`.
    pub cut_files: u64,
}

/// One entry of `cut`: `{path, rows_kept}`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CutFile {
    /// The file's path.
    pub path: String,
    /// The located diagnostics kept.
    pub rows_kept: u64,
}

/// One diagnostic. The members marked "only" in the schema are `None` where they do not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    /// The diagnostic's key (see the schema's Keys).
    pub key: String,
    /// `<label>@<anchor>`.
    pub code: String,
    /// The label.
    pub label: String,
    /// `security`, `integrity` or `shape`.
    pub class: String,
    /// The file's path: the only text of the change in the document.
    pub path: String,
    /// `line` or `file`.
    pub anchor: String,
    /// `new` or `old`; `None` at a file.
    pub side: Option<String>,
    /// The line on that side; `None` at a file.
    pub line: Option<u64>,
    /// 0, or the count of earlier diagnostics at the same place.
    pub occurrence: u64,
    /// `current` or `floor_only`.
    pub reading: String,
    /// `agent` or `person`.
    pub audience: String,
    /// Only on `hazardous_text` and `diff_suppressed`: the case the engine found.
    pub cases: Option<Vec<String>>,
    /// Ids of the changes that answer the label.
    pub repairs: Vec<String>,
    /// Ids of substitutes that do not answer it and that this engine raises a label for.
    pub not_accepted: Vec<String>,
    /// Only on `panic_path_added@line`.
    pub test_likely: Option<bool>,
    /// Only when `judge.state` is `different`.
    pub judge_sees: Option<bool>,
}

impl Diagnostics {
    /// Read a `deep-diff-forge.diagnostics.v0` document. A member the model does not know is
    /// ignored, as the schema's stability rule asks of readers; [`Diagnostics::to_json`] then
    /// returns `text` as given.
    ///
    /// # Errors
    /// [`Error::Document`] (with `pages` 0) for text that is not JSON, names another schema or
    /// lacks a member of the schema or has one of another type.
    pub fn from_json(text: &str) -> Result<Self, Error> {
        let value: Value = serde_json::from_str(text).map_err(|_| Error::Document {
            pages: 0,
            problem: "it is not JSON",
        })?;
        let mut document = read(&value).map_err(|problem| Error::Document { pages: 0, problem })?;
        text.clone_into(&mut document.bytes);
        Ok(document)
    }

    /// The document's bytes: the builder's for [`diagnostics`](crate::diagnostics), the rebuilt
    /// bytes that hash to `page.document_sha256` for [`Client::diagnostics`](crate::Client::diagnostics),
    /// the text given to [`Diagnostics::from_json`].
    #[must_use]
    pub fn to_json(&self) -> &str {
        &self.bytes
    }
}

/// The model of a parsed document; its bytes are left empty.
pub(crate) fn read(value: &Value) -> Result<Diagnostics, &'static str> {
    if value["schema"] != DIAGNOSTICS_SCHEMA {
        return Err("its schema is not deep-diff-forge.diagnostics.v0");
    }
    let tool = &value["tool"];
    let judge = &value["judge"];
    let counts = &value["counts"];
    let texts = value["texts"]
        .as_object()
        .ok_or("texts is not an object")?
        .iter()
        .map(|(id, text)| Ok((id.clone(), string(text)?)))
        .collect::<Result<_, &'static str>>()?;
    Ok(Diagnostics {
        input_sha256: string(&value["input_sha256"])?,
        tool: DiagnosticsTool {
            name: string(&tool["name"])?,
            version: string(&tool["version"])?,
            scorer_sha256: string(&tool["scorer_sha256"])?,
        },
        judge: DiagnosticsJudge {
            state: string(&judge["state"])?,
            ranker_sha256: optional(&judge["ranker_sha256"], string)?,
        },
        counts: DiagnosticsCounts {
            files: number(&counts["files"])?,
            diagnostics: number(&counts["diagnostics"])?,
            agent: number(&counts["agent"])?,
            person: number(&counts["person"])?,
            cut_files: number(&counts["cut_files"])?,
        },
        cut: list(&value["cut"])?
            .iter()
            .map(|c| {
                Ok(CutFile {
                    path: string(&c["path"])?,
                    rows_kept: number(&c["rows_kept"])?,
                })
            })
            .collect::<Result<_, &'static str>>()?,
        diagnostics: list(&value["diagnostics"])?
            .iter()
            .map(diagnostic)
            .collect::<Result<_, _>>()?,
        texts,
        bytes: String::new(),
    })
}

fn diagnostic(d: &Value) -> Result<Diagnostic, &'static str> {
    let only = |key: &str| d.get(key).filter(|v| !v.is_null());
    Ok(Diagnostic {
        key: string(&d["key"])?,
        code: string(&d["code"])?,
        label: string(&d["label"])?,
        class: string(&d["class"])?,
        path: string(&d["path"])?,
        anchor: string(&d["anchor"])?,
        side: optional(&d["side"], string)?,
        line: optional(&d["line"], number)?,
        occurrence: number(&d["occurrence"])?,
        reading: string(&d["reading"])?,
        audience: string(&d["audience"])?,
        cases: only("cases").map(strings).transpose()?,
        repairs: strings(&d["repairs"])?,
        not_accepted: strings(&d["not_accepted"])?,
        test_likely: only("test_likely").map(boolean).transpose()?,
        judge_sees: only("judge_sees").map(boolean).transpose()?,
    })
}

fn strings(v: &Value) -> Result<Vec<String>, &'static str> {
    list(v)?.iter().map(string).collect()
}

fn string(v: &Value) -> Result<String, &'static str> {
    v.as_str()
        .map(str::to_owned)
        .ok_or("a member that is a string is missing or of another type")
}

fn number(v: &Value) -> Result<u64, &'static str> {
    v.as_u64()
        .ok_or("a member that is a count is missing or of another type")
}

fn boolean(v: &Value) -> Result<bool, &'static str> {
    v.as_bool()
        .ok_or("a member that is a boolean is of another type")
}

fn list(v: &Value) -> Result<&Vec<Value>, &'static str> {
    v.as_array()
        .ok_or("a member that is a list is missing or of another type")
}

fn optional<T>(
    v: &Value,
    read: fn(&Value) -> Result<T, &'static str>,
) -> Result<Option<T>, &'static str> {
    if v.is_null() {
        Ok(None)
    } else {
        read(v).map(Some)
    }
}

/// A string in the byte form: `rank.v0`'s escapes, and every bidi control and invisible
/// character as `\u` escapes (a pair above U+FFFF).
fn text(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len() + 2);
    for c in deep_diff_forge_core::json_escape(value).chars() {
        if deep_diff_forge_core::is_unsafe_in_untrusted_text(c) {
            for unit in c.encode_utf16(&mut [0; 2]) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn texts(items: &[String]) -> String {
    let items: Vec<String> = items.iter().map(|s| text(s)).collect();
    format!("[{}]", items.join(", "))
}

fn block(items: &[String], open: char, close: char) -> String {
    if items.is_empty() {
        return format!("{open}{close}");
    }
    format!("{open}\n    {}\n  {close}", items.join(",\n    "))
}

/// The document's bytes in the byte form of `docs/reference/diagnostics.md`, written from the
/// model: the members it knows, in the schema's order.
pub(crate) fn write(d: &Diagnostics) -> String {
    use std::fmt::Write as _;
    let null = || "null".to_owned();
    let items: Vec<String> = d
        .diagnostics
        .iter()
        .map(|x| {
            let mut item = format!(
                "{{\"key\": {}, \"code\": {}, \"label\": {}, \"class\": {}, \"path\": {}, \"anchor\": {}, \"side\": {}, \"line\": {}, \"occurrence\": {}, \"reading\": {}, \"audience\": {}",
                text(&x.key),
                text(&x.code),
                text(&x.label),
                text(&x.class),
                text(&x.path),
                text(&x.anchor),
                x.side.as_deref().map_or_else(null, text),
                x.line.map_or_else(null, |n| n.to_string()),
                x.occurrence,
                text(&x.reading),
                text(&x.audience),
            );
            if let Some(cases) = &x.cases {
                let _ = write!(item, ", \"cases\": {}", texts(cases));
            }
            let _ = write!(
                item,
                ", \"repairs\": {}, \"not_accepted\": {}",
                texts(&x.repairs),
                texts(&x.not_accepted)
            );
            if let Some(likely) = x.test_likely {
                let _ = write!(item, ", \"test_likely\": {likely}");
            }
            if let Some(sees) = x.judge_sees {
                let _ = write!(item, ", \"judge_sees\": {sees}");
            }
            item.push('}');
            item
        })
        .collect();
    let cut: Vec<String> = d
        .cut
        .iter()
        .map(|c| {
            format!(
                "{{\"path\": {}, \"rows_kept\": {}}}",
                text(&c.path),
                c.rows_kept
            )
        })
        .collect();
    let named: Vec<String> = d
        .texts
        .iter()
        .map(|(id, sentence)| format!("{}: {}", text(id), text(sentence)))
        .collect();
    let c = &d.counts;
    format!(
        "{{\n  \"schema\": {},\n  \"input_sha256\": {},\n  \"tool\": {{\"name\": {}, \"version\": {}, \"scorer_sha256\": {}}},\n  \"judge\": {{\"state\": {}, \"ranker_sha256\": {}}},\n  \"counts\": {{\"files\": {}, \"diagnostics\": {}, \"agent\": {}, \"person\": {}, \"cut_files\": {}}},\n  \"cut\": {},\n  \"diagnostics\": {},\n  \"texts\": {}\n}}\n",
        text(DIAGNOSTICS_SCHEMA),
        text(&d.input_sha256),
        text(&d.tool.name),
        text(&d.tool.version),
        text(&d.tool.scorer_sha256),
        text(&d.judge.state),
        d.judge.ranker_sha256.as_deref().map_or_else(null, text),
        c.files,
        c.diagnostics,
        c.agent,
        c.person,
        c.cut_files,
        block(&cut, '[', ']'),
        block(&items, '[', ']'),
        block(&named, '{', '}'),
    )
}

#[cfg(unix)]
const METHOD: &str = "diff.diagnostics";
#[cfg(unix)]
const NEEDED: u32 = 3;

#[cfg(unix)]
fn too_old(daemon: Option<u64>) -> Error {
    Error::ProtocolTooOld {
        daemon: daemon.and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        needed: NEEDED,
        method: METHOD,
    }
}

/// `engine.initialize` with protocol 3: the protocol the daemon speaks, or
/// [`Error::ProtocolTooOld`] naming the newest it speaks.
#[cfg(unix)]
fn negotiate(call: &mut impl FnMut(&str, &Value) -> Result<Value, Error>) -> Result<u64, Error> {
    use serde_json::json;
    let daemon = match call("engine.initialize", &json!({"protocol": NEEDED})) {
        Ok(answer) => answer["protocol"].as_u64(),
        Err(Error::Rpc { code, .. }) if code == deep_diff_forge_daemon::PROTOCOL_UNSUPPORTED => {
            let newest = call("engine.initialize", &json!({}))?;
            return Err(too_old(newest["protocol"].as_u64()));
        }
        Err(other) => return Err(other),
    };
    match daemon {
        Some(n) if n >= u64::from(NEEDED) => Ok(n),
        other => Err(too_old(other)),
    }
}

/// What the pages so far have fixed: where the next starts, `total`, `document_sha256` and the
/// cursors seen.
#[cfg(unix)]
#[derive(Default)]
struct Paging {
    expected: u64,
    total: Option<u64>,
    digest: Option<String>,
    cursors: std::collections::BTreeSet<String>,
}

#[cfg(unix)]
impl Paging {
    /// Check one page against the contract; `Some(cursor)` for the next page, `None` at the end.
    fn next(&mut self, page: &Value, limit: u64) -> Result<Option<String>, &'static str> {
        let member = &page["page"];
        let (Some(first), Some(count), Some(total), Some(sha)) = (
            member["first"].as_u64(),
            member["count"].as_u64(),
            member["total"].as_u64(),
            member["document_sha256"].as_str(),
        ) else {
            return Err("a page has no page member of the contract");
        };
        let held = page["diagnostics"].as_array().map(Vec::len);
        if held.and_then(|n| u64::try_from(n).ok()) != Some(count) {
            return Err("a page's count is not the diagnostics it holds");
        }
        if *self.total.get_or_insert(total) != total {
            return Err("total changed between pages");
        }
        if *self.digest.get_or_insert_with(|| sha.to_owned()) != sha {
            return Err("document_sha256 changed between pages");
        }
        if first != self.expected {
            return Err("a page does not start where the last one ended");
        }
        if total > limit {
            return Err("total is more diagnostics than the patch has bytes");
        }
        if count == 0 && total > 0 {
            return Err("a page holds no diagnostic");
        }
        self.expected = first.saturating_add(count);
        if self.expected > total {
            return Err("a page goes past total");
        }
        match &member["next_cursor"] {
            Value::Null if self.expected == total => Ok(None),
            Value::Null => Err("the cursors ended before the document did"),
            Value::String(_) if self.expected == total => {
                Err("the cursors did not end with the document")
            }
            Value::String(cursor) if !self.cursors.insert(cursor.clone()) => {
                Err("a cursor came back")
            }
            Value::String(cursor) => Ok(Some(cursor.clone())),
            _ => Err("next_cursor is neither a string nor null"),
        }
    }
}

/// `diff.diagnostics` page by page through `call`, the document rebuilt and checked (engine
/// protocol 3; see [`Client::diagnostics`](crate::Client::diagnostics)).
#[cfg(unix)]
pub(crate) fn fetch(
    patch: &str,
    guards: crate::Guards,
    max_bytes: Option<usize>,
    mut call: impl FnMut(&str, &Value) -> Result<Value, Error>,
) -> Result<Diagnostics, Error> {
    use serde_json::json;
    let daemon = negotiate(&mut call)?;
    let mut params = json!({"patch": patch});
    if guards.0.files {
        params["require_files"] = json!(true);
    }
    if guards.0.hunks {
        params["require_hunks"] = json!(true);
    }
    if let Some(max_bytes) = max_bytes {
        params["max_bytes"] = json!(max_bytes);
    }
    // A document has fewer diagnostics than its patch has bytes, and every page holds one.
    let limit = u64::try_from(patch.len()).unwrap_or(u64::MAX);
    let mut pages: Vec<Value> = Vec::new();
    let mut paging = Paging::default();
    loop {
        let page = match call(METHOD, &params) {
            Ok(page) => page,
            Err(Error::Rpc { code, .. }) if code == deep_diff_forge_daemon::METHOD_NOT_FOUND => {
                return Err(too_old(Some(daemon)));
            }
            Err(Error::Rpc { code, message }) => {
                let message = without_patch_text(code, &message).to_owned();
                return Err(Error::Rpc { code, message });
            }
            Err(other) => return Err(other),
        };
        let next = paging
            .next(&page, limit)
            .map_err(|problem| Error::Document {
                pages: pages.len() + 1,
                problem,
            })?;
        pages.push(page);
        match next {
            Some(cursor) => params["cursor"] = json!(cursor),
            None => break,
        }
    }
    rebuild(&pages, paging.digest.as_deref()).map_err(|problem| Error::Document {
        pages: pages.len(),
        problem,
    })
}

/// The document of `pages`: the first page's header without `page`, every page's `cut` and
/// diagnostics in order and the union of `texts` (docs/reference/diagnostics.md, "Paging"),
/// written in the byte form; only when those bytes hash to `digest`.
#[cfg(unix)]
fn rebuild(pages: &[Value], digest: Option<&str>) -> Result<Diagnostics, &'static str> {
    let mut whole = pages.first().cloned().unwrap_or_default();
    if let Some(members) = whole.as_object_mut() {
        members.remove("page");
    }
    let mut cut = Vec::new();
    let mut items = Vec::new();
    let mut named = serde_json::Map::new();
    for page in pages {
        cut.extend(page["cut"].as_array().into_iter().flatten().cloned());
        items.extend(
            page["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
        for (id, text) in page["texts"].as_object().into_iter().flatten() {
            named.insert(id.clone(), text.clone());
        }
    }
    whole["cut"] = Value::Array(cut);
    whole["diagnostics"] = Value::Array(items);
    whole["texts"] = Value::Object(named);
    let mut document = read(&whole)?;
    document.bytes = write(&document);
    if digest != Some(deep_diff_forge_core::sha256_hex(document.bytes.as_bytes()).as_str()) {
        return Err("the rebuilt document does not hash to document_sha256");
    }
    Ok(document)
}

/// The message of a daemon error for a diagnostics call: a parse failure (error 4, or 3 for a
/// header that is not UTF-8) quotes the line that failed after its first `": "`; it is cut there,
/// so an error carries where the patch failed and no text of it.
#[cfg(unix)]
fn without_patch_text(code: i64, message: &str) -> &str {
    let parse = [crate::exit::INVALID_INPUT, crate::exit::IO].map(i64::from);
    if parse.contains(&code) {
        message.split_once(": ").map_or(message, |(head, _)| head)
    } else {
        message
    }
}
