//! `Client` against a real daemon, run in-process on a private socket: the daemon's
//! `diff.compute` and the facade's `review` give the same document, and errors keep their codes.
//! Unix only, as `Client` is.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test: drives real processes, the environment and wall-clock time"
)]

use deep_diff_forge_sdk::{Client, Error, Guards, diagnostics, review};
use deep_diff_forge_testkit::Scratch;
use serde_json::{Value, json};

#[test]
fn the_client_and_the_facade_agree_with_a_live_daemon() {
    let dir = Scratch::new("sdk");
    let socket = dir.join("d.sock");
    let location = deep_diff_forge_daemon::SocketLocation::at(&socket);
    let server = std::thread::spawn(move || deep_diff_forge_daemon::run_server(&location));
    let client = Client::at(&socket);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.call("daemon.health", &Value::Null).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never answered"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let patch = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
    let remote = client
        .call("diff.compute", &json!({"patch": patch}))
        .unwrap();
    let local: Value =
        serde_json::from_str(&review(patch, Guards::none()).unwrap().to_json()).unwrap();
    assert_eq!(remote, local);

    let refused = client
        .call("diff.compute", &json!({"patch": "", "require_files": true}))
        .unwrap_err();
    assert_eq!(
        refused,
        Error::Rpc {
            code: 7,
            message: "refused: 0 files in input (--require-files)".into()
        }
    );
    let unknown = client.call("no.such", &Value::Null).unwrap_err();
    assert!(
        matches!(unknown, Error::Rpc { code: -32601, .. }),
        "{unknown}"
    );
    // The daemon answers an invalid request with id null; the client still reports its error.
    let invalid = client.call("", &Value::Null).unwrap_err();
    assert!(
        matches!(invalid, Error::Rpc { code: -32600, .. }),
        "{invalid}"
    );

    assert_eq!(
        client.call("daemon.shutdown", &Value::Null).unwrap()["stopping"],
        true
    );
    server.join().unwrap().unwrap();
}

/// Six files of process spawns (three cut at 32 rows) with a hidden character in each path, and
/// a line holding a bidi control: a document of many pages at the smallest budget.
fn paging_patch() -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for f in 0..6 {
        let path = format!("src/m{f}/{}\u{200b}.rs", "n".repeat(30 * f));
        let lines = 25 + 5 * f;
        let _ = write!(
            text,
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{lines} @@\n"
        );
        for i in 0..lines {
            let _ = writeln!(text, "+std::process::Command::new(\"p{i}\");");
        }
    }
    text.push_str("diff --git a/x.py b/x.py\n--- a/x.py\n+++ b/x.py\n@@ -1 +1 @@\n-a = 1\n+a = eval(s)  # \u{202e}\n");
    text
}

/// A daemon run in-process on a private socket, and a client of it.
fn live_daemon(
    name: &str,
) -> (
    Scratch,
    Client,
    std::thread::JoinHandle<std::io::Result<()>>,
) {
    let dir = Scratch::new(name);
    let socket = dir.join("d.sock");
    let location = deep_diff_forge_daemon::SocketLocation::at(&socket);
    let server = std::thread::spawn(move || deep_diff_forge_daemon::run_server(&location));
    let client = Client::at(&socket);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.call("daemon.health", &Value::Null).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never answered"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    (dir, client, server)
}

/// F5b: `Client::diagnostics` against this tree's daemon pages at the smallest budget and at the
/// default, and rebuilds the builder's bytes (what `--stdin-patch --diagnostics --json` prints)
/// from the pages; a refusal keeps its code; a parse failure's message holds no text of the patch.
#[test]
fn the_client_rebuilds_the_builders_document_from_the_pages() {
    let (_dir, client, server) = live_daemon("sdk-diagnostics");
    let patch = paging_patch();
    let local = diagnostics(&review(&patch, Guards::none()).unwrap());
    let mut calls = 0;
    for max_bytes in [Some(4096), None] {
        let remote = client
            .diagnostics(&patch, Guards::none(), max_bytes)
            .unwrap();
        assert_eq!(remote.to_json(), local.to_json(), "at {max_bytes:?}");
        assert_eq!(remote, local);
        calls += 1;
    }
    assert_eq!(calls, 2);
    assert!(local.diagnostics.len() > 40 && !local.cut.is_empty());
    let refused = client
        .diagnostics("", Guards::none().require_files(), None)
        .unwrap_err();
    assert_eq!(
        refused,
        Error::Rpc {
            code: 7,
            message: "refused: 0 files in input (--require-files)".into()
        }
    );
    let marker = "zq7MARKER\u{202e}";
    for broken in [
        format!("--- a/x\n+++ b/x\n@@ -1,{marker} +1 @@\n-a\n+b\n"),
        format!("--- a/x\n+++ b/x\n+{marker}\n"),
    ] {
        let error = client
            .diagnostics(&broken, Guards::none(), None)
            .unwrap_err();
        assert!(matches!(error, Error::Rpc { code: 4, .. }), "{error}");
        assert!(!error.to_string().contains("zq7MARKER"), "{error}");
        assert!(!error.to_string().contains('\u{202e}'), "{error}");
    }
    let empty = client.diagnostics("", Guards::none(), None).unwrap();
    assert_eq!(empty.counts.diagnostics, 0);
    assert_eq!(
        client.call("daemon.shutdown", &Value::Null).unwrap()["stopping"],
        true
    );
    server.join().unwrap().unwrap();
}

/// A daemon stand-in on a private socket: each connection gets one answer line, `answer(request)`
/// with the request's id. It serves until the test ends.
fn stub(name: &str, answer: impl Fn(&Value) -> Value + Send + 'static) -> (Scratch, Client) {
    use std::io::{BufRead as _, Write as _};
    let dir = Scratch::new(name);
    let socket = dir.join("stub.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut line = String::new();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let request: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            let mut response = answer(&request);
            response["id"] = request["id"].clone();
            let _ = writeln!(stream, "{response}");
        }
    });
    (dir, Client::at(&socket))
}

const TRANSCRIPT: &str = include_str!("../../../fixtures/protocol/v2-transcript.jsonl");

/// The published 0.10.0 daemon as its transcript recorded it (F7.d1): the recorded answer to a
/// request of the same method and the same `protocol` param.
fn old_daemon(request: &Value) -> Value {
    for line in TRANSCRIPT.lines().skip(1) {
        let record: Value = serde_json::from_str(line).unwrap();
        // The transcript records malformed requests too; they are no request of this client's.
        let asked: Value =
            serde_json::from_str(record["request"].as_str().unwrap()).unwrap_or_default();
        if asked["method"] == request["method"]
            && asked["params"]["protocol"] == request["params"]["protocol"]
        {
            return serde_json::from_str(record["response"].as_str().unwrap()).unwrap();
        }
    }
    json!({"jsonrpc": "2.0", "error": {"code": -32601, "message": "not in the transcript"}})
}

/// F5b: against a daemon of release 0.10.0 (a stub answering from the recorded transcript),
/// `engine.initialize` with protocol 3 is error 2 and the client names the daemon too old; a
/// daemon that serves the protocol but not the method (-32601) is named the same way. Nothing is
/// built from the older daemon's `diff.rank`: no such request is sent.
#[test]
fn an_old_daemon_is_named_never_worked_around() {
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&asked);
    let (_dir, client) = stub("sdk-old-daemon", move |request| {
        seen.lock().unwrap().push(request["method"].clone());
        old_daemon(request)
    });
    let error = client.diagnostics(
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
        Guards::none(),
        None,
    );
    assert_eq!(
        error,
        Err(Error::ProtocolTooOld {
            daemon: 2,
            needed: 3,
            method: "diff.diagnostics"
        })
    );
    assert_eq!(
        error.unwrap_err().to_string(),
        "protocol too old: diff.diagnostics needs engine protocol 3; the daemon speaks 2"
    );
    assert_eq!(
        *asked.lock().unwrap(),
        [json!("engine.initialize"), json!("engine.initialize")]
    );
    let (_dir2, skips) = stub("sdk-skipping-daemon", |request| {
        if request["method"] == "engine.initialize" {
            json!({"jsonrpc": "2.0", "result": {"protocol": 3}})
        } else {
            old_daemon(request)
        }
    });
    assert_eq!(
        skips.diagnostics("", Guards::none(), None),
        Err(Error::ProtocolTooOld {
            daemon: 3,
            needed: 3,
            method: "diff.diagnostics"
        })
    );
}

/// The pages this tree's pager gives `patch` at 4096 bytes, each a `diff.diagnostics` result.
fn real_pages(patch: &str) -> Vec<Value> {
    let review = deep_diff_forge_patch::SealedReview::new(
        patch.as_bytes(),
        deep_diff_forge_patch::InputGuards {
            files: false,
            hunks: false,
        },
        env!("CARGO_PKG_VERSION"),
    )
    .unwrap();
    let (mut pages, mut cursor) = (Vec::new(), None);
    loop {
        let page = deep_diff_forge_daemon::pages::page(
            &review,
            &deep_diff_forge_daemon::views::Page {
                max_bytes: 4096,
                cursor,
            },
        )
        .unwrap();
        cursor = page["page"]["next_cursor"].as_str().map(str::to_owned);
        pages.push(page);
        if cursor.is_none() {
            return pages;
        }
    }
}

/// A protocol-3 stand-in that answers page `k` of `pages` to the request whose cursor ends in
/// `.k`-th position (the first page without a cursor), after `tamper` has changed it.
fn serving(pages: Vec<Value>, tamper: fn(usize, &mut Value)) -> impl Fn(&Value) -> Value {
    move |request: &Value| {
        if request["method"] == "engine.initialize" {
            return json!({"jsonrpc": "2.0", "result": {"protocol": 3}});
        }
        let index = request["params"]["cursor"].as_str().map_or(0, |cursor| {
            pages
                .iter()
                .position(|p| p["page"]["next_cursor"] == cursor)
                .map_or(usize::MAX, |i| i + 1)
        });
        let Some(page) = pages.get(index) else {
            return json!({"jsonrpc": "2.0", "error": {"code": -32602, "message": "invalid cursor: it is not a dg0 cursor (1 bytes)"}});
        };
        let mut page = page.clone();
        tamper(index, &mut page);
        json!({"jsonrpc": "2.0", "result": page})
    }
}

/// A tampering of the pages, the page at which the call must end (`usize::MAX`: the last), and
/// the problem it must name.
type Case = (fn(usize, &mut Value), usize, &'static str);

/// F5b, ruling R4: the paging call is bounded and believes no page that breaks the contract. A
/// daemon whose cursors do not end, that repeats a cursor, changes `total` or
/// `document_sha256` between pages, or sends a page that does not start where the last ended, or
/// pages that do not rebuild the hashed bytes, ends the call with a named error at a stated page.
#[test]
fn a_daemon_that_breaks_paging_ends_the_call_with_a_named_error() {
    let patch = paging_patch();
    let pages = real_pages(&patch);
    assert!(pages.len() >= 4, "{} pages", pages.len());
    let (_ok, control) = stub("sdk-pages-ok", serving(pages.clone(), |_, _| {}));
    assert_eq!(
        control
            .diagnostics(&patch, Guards::none(), None)
            .unwrap()
            .to_json(),
        diagnostics(&review(&patch, Guards::none()).unwrap()).to_json()
    );
    let cases: [Case; 6] = [
        (
            |i, p| {
                if i == 2 {
                    p["page"]["next_cursor"] = json!("dg0.never.ends.1");
                    p["page"]["count"] = json!(0);
                    p["diagnostics"] = json!([]);
                }
            },
            3,
            "a page holds no diagnostic",
        ),
        (
            |i, p| {
                if i > 0 {
                    p["page"]["total"] = json!(p["page"]["total"].as_u64().unwrap() + 1);
                }
            },
            2,
            "total changed between pages",
        ),
        (
            |i, p| {
                if i == 1 {
                    p["page"]["document_sha256"] = json!("0".repeat(64));
                }
            },
            2,
            "document_sha256 changed between pages",
        ),
        (
            |i, p| {
                if i == 1 {
                    p["page"]["first"] = json!(p["page"]["first"].as_u64().unwrap() - 1);
                }
            },
            2,
            "a page does not start where the last one ended",
        ),
        (
            |i, p| {
                if i == 1 {
                    p["page"]["next_cursor"] = json!(null);
                }
            },
            2,
            "the cursors ended before the document did",
        ),
        (
            |i, p| {
                if i == 1 {
                    p["diagnostics"][0]["line"] = json!(99_999);
                }
            },
            usize::MAX,
            "the rebuilt document does not hash to document_sha256",
        ),
    ];
    for (k, (tamper, at, problem)) in cases.into_iter().enumerate() {
        let (_dir, client) = stub(&format!("sdk-pages-{k}"), serving(pages.clone(), tamper));
        let error = client
            .diagnostics(&patch, Guards::none(), None)
            .unwrap_err();
        let at = if at == usize::MAX { pages.len() } else { at };
        assert_eq!(error, Error::Document { pages: at, problem }, "case {k}");
    }
}

/// F5b, ruling R4: a cursor that comes back, and cursors that do not end with the document, end
/// the call with a named error at the page that broke the rule.
#[test]
fn cursors_that_repeat_or_never_end_end_the_call() {
    let patch = paging_patch();
    let pages = real_pages(&patch);
    let last = pages.len() - 1;
    // A cursor that comes back: the third page names the second's cursor again.
    let repeat = pages.clone();
    let repeated = move |request: &Value| {
        let mut answer = serving(repeat.clone(), |_, _| {})(request);
        if answer["result"]["page"]["first"] == repeat[2]["page"]["first"] {
            answer["result"]["page"]["next_cursor"] = repeat[1]["page"]["next_cursor"].clone();
        }
        answer
    };
    let (_repeat, client) = stub("sdk-pages-repeat", repeated);
    assert_eq!(
        client
            .diagnostics(&patch, Guards::none(), None)
            .unwrap_err(),
        Error::Document {
            pages: 3,
            problem: "a cursor came back"
        }
    );
    // Cursors that do not end with the document: the last page names a next one.
    let endless = pages.clone();
    let never = move |request: &Value| {
        let mut answer = serving(endless.clone(), |_, _| {})(request);
        if answer["result"]["page"]["first"] == endless[last]["page"]["first"] {
            answer["result"]["page"]["next_cursor"] =
                json!("dg0.0000000000000000.0000000000000000.1");
        }
        answer
    };
    let (_endless, client) = stub("sdk-pages-endless", never);
    assert_eq!(
        client
            .diagnostics(&patch, Guards::none(), None)
            .unwrap_err(),
        Error::Document {
            pages: last + 1,
            problem: "the cursors did not end with the document"
        }
    );
}
