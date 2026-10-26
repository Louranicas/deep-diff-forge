// Module vault note (bidirectional): obsidian://open?vault=deep-diff-forge.vault&file=Implementation%2FModules%2Fdeep-diff-forge-cli--mcp
// DDF slice D: `deep-diff-forge mcp`, a read-only Model Context Protocol server over stdio.
// Design/status: docs/ROADMAP.md#status-2026-10-26 and README "MCP server".
//! A dual-era MCP server (specification 2026-07-28, with the 2025-11-25 `initialize` handshake for
//! legacy clients) whose tools are the daemon's own methods, so an MCP client gets exactly what the
//! CLI and the daemon give: `review_patch` is `diff.compute`, `rank_patch` is `diff.plan`. Every
//! tool is read-only; nothing it does writes a file, a store or a session.
//!
//! Framing is the stdio binding's: one JSON-RPC message per line, nothing else on stdout. A line
//! longer than [`LINE_CAP`] is refused and skipped, so input can never grow memory past the cap.

use deep_diff_forge_core::exit::{EXIT_CODES, lookup};
use deep_diff_forge_daemon::{
    Engine, INPUT_CONTRACT_REFUSED, INVALID_PARAMS, METHOD_NOT_FOUND, PARSE_ERROR,
    PATCH_PARSE_FAILED, Request, RpcError, dispatch,
};
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

/// The modern revision this server implements (per-request `_meta`, `server/discover`).
pub const MODERN: &str = "2026-07-28";
/// Legacy revisions served through the `initialize` handshake, newest first.
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
/// The longest request line accepted: the patch byte budget plus room for the JSON around it.
pub const LINE_CAP: usize = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET + 1024 * 1024;

const PROTOCOL_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";
/// MCP's `UnsupportedProtocolVersion` error code.
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const INSTRUCTIONS: &str = "deep-diff-forge reviews unified or Git patches. review_patch returns the \
deep-diff-forge.review.v0 document (files, hunks, lines, input seal); rank_patch orders files by \
review risk; explain_exit_code and exit_code_registry describe the CLI's exit codes. Every tool is \
read-only.";

/// Which era a request is served in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy(String),
}

/// The per-process state: only the legacy handshake's negotiated version. Modern requests carry
/// everything they need, as the 2026-07-28 statelessness rule requires.
#[derive(Debug, Default)]
pub struct Server {
    legacy: Option<String>,
}

impl Server {
    /// Answer one line: `Some(response)` for a request, `None` for a notification.
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let mut message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(e) => {
                return Some(error(
                    &Value::Null,
                    PARSE_ERROR,
                    &format!("parse error: {e}"),
                    None,
                ));
            }
        };
        let Some(object) = message.as_object_mut() else {
            return Some(error(
                &Value::Null,
                -32600,
                "a message must be a JSON object",
                None,
            ));
        };
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = object.get("id").cloned();
        let valid_id = id
            .as_ref()
            .filter(|id| id.is_string() || id.is_i64() || id.is_u64());
        // Only a JSON-RPC 2.0 envelope is acted on, before any state can change (council sdc1).
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return valid_id
                .map(|id| error(id, -32600, "jsonrpc must be \"2.0\"", None))
                .or_else(|| {
                    id.is_some()
                        .then(|| error(&Value::Null, -32600, "jsonrpc must be \"2.0\"", None))
                });
        }
        let Some(id) = valid_id.cloned() else {
            // A notification (or a malformed id): never answered. `notifications/initialized`
            // and `notifications/cancelled` need no action from a synchronous server.
            return (id.is_some() && method.is_some()).then(|| {
                error(
                    &Value::Null,
                    -32600,
                    "id must be a string or an integer",
                    None,
                )
            });
        };
        let Some(method) = method else {
            return Some(error(&id, -32600, "a request needs a method", None));
        };
        // Moved, not cloned: the patch inside is the largest thing a request carries (council sdc1).
        let params = object.remove("params").unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Some(error(&id, INVALID_PARAMS, "params must be an object", None));
        }
        Some(self.request(&id, &method, params))
    }

    fn request(&mut self, id: &Value, method: &str, params: Value) -> Value {
        if method == "initialize" {
            return self.initialize(id, &params);
        }
        let era = match era_of(&params, self.legacy.as_deref()) {
            Ok(era) => era,
            Err(response) => return response(id),
        };
        let result = match method {
            "server/discover" if era == Era::Modern => Ok(json!({
                "supportedVersions": [MODERN],
                "capabilities": {"tools": {}},
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => call_tool(params),
            _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
        };
        match result {
            Ok(mut value) => {
                if era == Era::Modern {
                    value["resultType"] = json!("complete");
                    value["_meta"] = json!({SERVER_INFO_KEY: server_info()});
                }
                json!({"jsonrpc": "2.0", "id": id, "result": value})
            }
            Err((code, message)) => error(id, code, &message, None),
        }
    }

    /// The legacy handshake: echo a legacy version the client asked for, or offer the newest.
    fn initialize(&mut self, id: &Value, params: &Value) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = LEGACY
            .iter()
            .find(|v| **v == requested)
            .copied()
            .unwrap_or(LEGACY[0]);
        self.legacy = Some(version.to_owned());
        json!({"jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS,
        }})
    }
}

type Reject = Box<dyn Fn(&Value) -> Value>;

/// The era of one request: modern when its `_meta` names a protocol version, legacy when the
/// handshake happened and it names none.
fn era_of(params: &Value, legacy: Option<&str>) -> Result<Era, Reject> {
    let meta = params.get("_meta").and_then(Value::as_object);
    match meta.and_then(|m| m.get(PROTOCOL_VERSION_KEY)) {
        Some(Value::String(version)) if version == MODERN => {
            if meta.is_some_and(|m| m.get(CLIENT_CAPABILITIES_KEY).is_some_and(Value::is_object)) {
                Ok(Era::Modern)
            } else {
                Err(Box::new(|id: &Value| {
                    error(id, INVALID_PARAMS, &format!("_meta needs {CLIENT_CAPABILITIES_KEY}"), None)
                }))
            }
        }
        Some(requested) => {
            let requested = requested.clone();
            Err(Box::new(move |id: &Value| {
                let mut supported = vec![MODERN];
                supported.extend(LEGACY);
                error(
                    id,
                    UNSUPPORTED_PROTOCOL_VERSION,
                    "Unsupported protocol version",
                    Some(json!({"supported": supported, "requested": requested})),
                )
            }))
        }
        None => legacy.map(|v| Era::Legacy(v.to_owned())).ok_or_else(|| -> Reject {
            Box::new(|id: &Value| {
                error(
                    id,
                    INVALID_PARAMS,
                    &format!("_meta needs {PROTOCOL_VERSION_KEY} ({MODERN}), or send initialize first"),
                    None,
                )
            })
        }),
    }
}

fn server_info() -> Value {
    json!({"name": "deep-diff-forge", "version": env!("CARGO_PKG_VERSION")})
}

fn error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

const READ_ONLY: &str = r#"{"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}"#;

/// The tools, in a fixed order.
fn tools() -> Value {
    let read_only: Value = serde_json::from_str(READ_ONLY).unwrap_or_default();
    let patch = json!({"type": "string", "description": "A unified or Git patch (the bytes `deep-diff-forge --stdin-patch` reads)."});
    json!([
        {
            "name": "review_patch",
            "title": "Review a patch",
            "description": "Parse a patch into the deep-diff-forge.review.v0 document: files, hunks and lines, sealed over the patch bytes. Identical to `deep-diff-forge --stdin-patch --json`. require_files / require_hunks refuse an empty or change-less patch, as the CLI's --require-files / --require-hunks do.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "patch": patch,
                    "require_files": {"type": "boolean", "description": "Refuse a patch with 0 files."},
                    "require_hunks": {"type": "boolean", "description": "Refuse a patch with 0 hunks or 0 changed lines."},
                },
                "required": ["patch"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "rank_patch",
            "title": "Rank a patch's files by review risk",
            "description": "Order a patch's files by review risk (the daemon's diff.plan).",
            "inputSchema": {"type": "object", "properties": {"patch": patch}, "required": ["patch"], "additionalProperties": false},
            "annotations": read_only,
        },
        {
            "name": "explain_exit_code",
            "title": "Explain a deep-diff-forge exit code",
            "description": "Name and describe one exit code of the CLI. The council subcommands use their own family.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "integer"},
                    "family": {"enum": ["engine", "council"], "description": "Defaults to engine."},
                },
                "required": ["code"],
                "additionalProperties": false,
            },
            "annotations": read_only,
        },
        {
            "name": "exit_code_registry",
            "title": "List deep-diff-forge exit codes",
            "description": "Every exit code the CLI documents, by family.",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": read_only,
        },
    ])
}

/// `tools/call`. An unknown tool or malformed call is a protocol error; a domain refusal (a parse
/// failure, a broken input contract, an undocumented code) is a tool execution error the model
/// can act on.
fn call_tool(mut params: Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "tools/call needs a name".to_owned()))?
        .to_owned();
    let arguments = match params.as_object_mut().and_then(|p| p.remove("arguments")) {
        None => Map::new(),
        Some(Value::Object(arguments)) => arguments,
        Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_owned())),
    };
    let known = tools();
    let spec = known
        .as_array()
        .and_then(|tools| tools.iter().find(|t| t["name"] == name.as_str()))
        .ok_or((INVALID_PARAMS, format!("Unknown tool: {name}")))?;
    if let Some(extra) = arguments
        .keys()
        .find(|k| spec["inputSchema"]["properties"].get(k.as_str()).is_none())
    {
        return Ok(tool_error(&format!("unknown argument: {extra}")));
    }
    Ok(match name.as_str() {
        "review_patch" => daemon_tool("diff.compute", arguments),
        "rank_patch" => daemon_tool("diff.plan", arguments),
        "explain_exit_code" => explain(&arguments),
        _ => structured(
            json!({"codes": EXIT_CODES.iter().map(|e| json!({"family": e.family, "code": e.code, "name": e.name, "meaning": e.meaning})).collect::<Vec<_>>()}),
        ),
    })
}

/// Run a daemon method in-process: the same dispatcher, so the same answer. As on the socket
/// (`process_line`), a panic inside it is contained and reported, never fatal to the server.
fn daemon_tool(method: &str, arguments: Map<String, Value>) -> Value {
    let request = Request {
        jsonrpc: "2.0".into(),
        id: json!(1),
        method: method.into(),
        params: Value::Object(arguments),
    };
    let outcome = std::panic::catch_unwind(|| dispatch(&request, &mut Engine::new()));
    match outcome {
        Ok(Ok(value)) => structured(value),
        Ok(Err(RpcError { code, message })) => tool_error(&match code {
            INPUT_CONTRACT_REFUSED | PATCH_PARSE_FAILED | INVALID_PARAMS => message,
            other => format!("daemon error {other}: {message}"),
        }),
        Err(_) => tool_error("internal error"),
    }
}

fn explain(arguments: &Map<String, Value>) -> Value {
    let family = arguments
        .get("family")
        .and_then(Value::as_str)
        .unwrap_or("engine");
    let Some(code) = arguments.get("code").and_then(Value::as_i64) else {
        return tool_error("code must be an integer");
    };
    let entry = i32::try_from(code)
        .ok()
        .and_then(|code| lookup(family, code));
    entry.map_or_else(
        || {
            tool_error(&format!(
                "exit code {code} is not documented for the {family} family"
            ))
        },
        |e| {
            structured(
                json!({"family": e.family, "code": e.code, "name": e.name, "meaning": e.meaning}),
            )
        },
    )
}

/// A successful result: the value as `structuredContent`, and as text for clients without it.
fn structured(value: Value) -> Value {
    let text = value.to_string();
    let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
    result["structuredContent"] = value;
    result
}

fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// Read one line of at most `cap` bytes. `Ok(None)` at end of input; an over-long line is drained
/// to its newline and reported as `Ok(Some(Err(())))`.
pub(crate) fn read_line(
    input: &mut impl BufRead,
    cap: usize,
) -> std::io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut line = Vec::new();
    let mut over = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if line.is_empty() && !over {
                None
            } else if over {
                Some(Err(()))
            } else {
                Some(Ok(line))
            });
        }
        let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
            Some(at) => (&available[..at], at + 1),
            None => (available, available.len()),
        };
        let found = done > chunk.len();
        if !over {
            if line.len() + chunk.len() > cap {
                over = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        input.consume(done);
        if found {
            return Ok(Some(if over { Err(()) } else { Ok(line) }));
        }
    }
}

/// Serve stdin until it closes. Each response is one line on stdout; diagnostics go to stderr.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    serve_with(input, output, LINE_CAP)
}

/// [`serve`] with an explicit line cap.
fn serve_with(
    input: &mut impl BufRead,
    output: &mut impl Write,
    cap: usize,
) -> std::io::Result<()> {
    let mut server = Server::default();
    while let Some(line) = read_line(input, cap)? {
        let response = match line {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) if text.trim().is_empty() => continue,
                Ok(text) => server.handle_line(&text),
                Err(_) => Some(error(
                    &Value::Null,
                    PARSE_ERROR,
                    "a message must be UTF-8",
                    None,
                )),
            },
            Err(()) => Some(error(
                &Value::Null,
                -32600,
                &format!("message longer than {cap} bytes"),
                None,
            )),
        };
        if let Some(response) = response {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modern(method: &str, extra: &Value) -> String {
        let mut params =
            json!({"_meta": {PROTOCOL_VERSION_KEY: MODERN, CLIENT_CAPABILITIES_KEY: {}}});
        if let Some(extra) = extra.as_object() {
            for (k, v) in extra {
                params[k] = v.clone();
            }
        }
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string()
    }

    /// Run `f` on a thread and fail if it does not finish in `seconds`: a reading loop that never
    /// ends is a failure here, not a hung test.
    fn within<T: Send + 'static>(seconds: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || send.send(f()));
        receive
            .recv_timeout(std::time::Duration::from_secs(seconds))
            .expect("did not finish: a reading loop never ended")
    }

    #[test]
    fn read_line_assembles_lines_across_small_reads() {
        within(5, || {
            let data: &[u8] = b"abcdefg\nhi\n";
            let mut input = std::io::BufReader::with_capacity(2, data);
            assert_eq!(
                read_line(&mut input, 16).unwrap(),
                Some(Ok(b"abcdefg".to_vec()))
            );
            assert_eq!(read_line(&mut input, 16).unwrap(), Some(Ok(b"hi".to_vec())));
            assert_eq!(read_line(&mut input, 16).unwrap(), None);
            let mut input = std::io::BufReader::with_capacity(2, data);
            assert_eq!(read_line(&mut input, 6).unwrap(), Some(Err(())));
            assert_eq!(read_line(&mut input, 6).unwrap(), Some(Ok(b"hi".to_vec())));
        });
    }

    #[test]
    fn an_over_long_line_is_refused_and_serving_continues() {
        let input = format!("{}\n{}\n", "x".repeat(300), modern("ping", &json!({})));
        let text = within(5, move || {
            let mut out = Vec::new();
            serve_with(&mut input.as_bytes(), &mut out, 256).unwrap();
            String::from_utf8(out).unwrap()
        });
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(
            (lines[0]["error"]["code"].as_i64(), lines[0]["id"].is_null()),
            (Some(-32600), true)
        );
        assert_eq!(lines[1]["id"], 1);
    }

    #[test]
    fn the_readme_states_the_line_cap() {
        let mib = 1024 * 1024;
        let budget = deep_diff_forge_patch::DEFAULT_BYTE_BUDGET;
        assert_eq!((budget / mib, LINE_CAP - budget), (64, mib));
        assert!(
            include_str!("../../../README.md")
                .contains("a request line is capped at the 64 MiB patch budget plus 1 MiB")
        );
    }

    #[test]
    fn read_line_caps_and_drains() {
        within(5, read_line_cases);
    }

    fn read_line_cases() {
        let mut input = &b"abc\ntoo-long-line\nok\nend"[..];
        assert_eq!(read_line(&mut input, 4).unwrap(), Some(Ok(b"abc".to_vec())));
        assert_eq!(read_line(&mut input, 4).unwrap(), Some(Err(())));
        assert_eq!(read_line(&mut input, 4).unwrap(), Some(Ok(b"ok".to_vec())));
        assert_eq!(read_line(&mut input, 4).unwrap(), Some(Ok(b"end".to_vec())));
        assert_eq!(read_line(&mut input, 4).unwrap(), None);
        let mut exact = &b"abcd\nabcde"[..];
        assert_eq!(
            read_line(&mut exact, 4).unwrap(),
            Some(Ok(b"abcd".to_vec()))
        );
        assert_eq!(read_line(&mut exact, 4).unwrap(), Some(Err(())));
    }

    #[test]
    fn modern_requests_carry_their_version_and_capabilities() {
        let mut server = Server::default();
        let v = server
            .handle_line(&modern("server/discover", &json!({})))
            .unwrap();
        assert_eq!(v["result"]["supportedVersions"], json!([MODERN]));
        assert_eq!(
            (
                v["result"]["resultType"].as_str(),
                v["result"]["_meta"][SERVER_INFO_KEY]["name"].as_str()
            ),
            (Some("complete"), Some("deep-diff-forge"))
        );
        let no_caps = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": {PROTOCOL_VERSION_KEY: MODERN}}});
        assert_eq!(
            server.handle_line(&no_caps.to_string()).unwrap()["error"]["code"],
            INVALID_PARAMS
        );
        let old = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {"_meta": {PROTOCOL_VERSION_KEY: "1900-01-01", CLIENT_CAPABILITIES_KEY: {}}}});
        let v = server.handle_line(&old.to_string()).unwrap();
        assert_eq!(
            v["error"]["code"], -32022,
            "MCP's UnsupportedProtocolVersion"
        );
        assert_eq!(v["error"]["data"]["supported"][0], MODERN);
        assert_eq!(v["error"]["data"]["requested"], "1900-01-01");
        let bare = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"});
        assert_eq!(
            server.handle_line(&bare.to_string()).unwrap()["error"]["code"],
            INVALID_PARAMS
        );
    }

    #[test]
    fn the_legacy_handshake_negotiates_and_then_serves() {
        let mut server = Server::default();
        let init = |v: &str| {
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": v, "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}).to_string()
        };
        assert_eq!(
            server.handle_line(&init("2025-06-18")).unwrap()["result"]["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(
            server.handle_line(&init("2024-01-01")).unwrap()["result"]["protocolVersion"],
            LEGACY[0]
        );
        assert_eq!(
            server.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        let v = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 4);
        assert!(
            v["result"].get("resultType").is_none(),
            "legacy results keep the legacy shape"
        );
        assert_eq!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"server/discover"}"#)
                .unwrap()["error"]["code"],
            METHOD_NOT_FOUND
        );
    }

    #[test]
    fn every_tool_is_read_only_and_listed_in_a_fixed_order() {
        let tools = tools();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "review_patch",
                "rank_patch",
                "explain_exit_code",
                "exit_code_registry"
            ]
        );
        for tool in tools.as_array().unwrap() {
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool}");
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        }
    }

    #[test]
    fn tool_calls_separate_protocol_errors_from_tool_errors() {
        let mut server = Server::default();
        let call = |server: &mut Server, args: Value| {
            server.handle_line(&modern("tools/call", &args)).unwrap()
        };
        let v = call(
            &mut server,
            json!({"name": "review_patch", "arguments": {"patch": "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n"}}),
        );
        assert_eq!(
            (
                v["result"]["isError"].as_bool(),
                v["result"]["structuredContent"]["schema"].as_str()
            ),
            (Some(false), Some("deep-diff-forge.review.v0"))
        );
        let v = call(
            &mut server,
            json!({"name": "review_patch", "arguments": {"patch": "", "require_files": true}}),
        );
        assert_eq!(
            (
                v["result"]["isError"].as_bool(),
                v["result"]["content"][0]["text"].as_str()
            ),
            (
                Some(true),
                Some("refused: 0 files in input (--require-files)")
            )
        );
        let v = call(
            &mut server,
            json!({"name": "review_patch", "arguments": {"patch": "x", "extra": 1}}),
        );
        assert_eq!(v["result"]["isError"], true);
        let v = call(&mut server, json!({"name": "rank_patch", "arguments": {}}));
        assert_eq!(v["result"]["isError"], true);
        let v = call(
            &mut server,
            json!({"name": "explain_exit_code", "arguments": {"code": 7, "family": "council"}}),
        );
        assert_eq!(v["result"]["structuredContent"]["name"], "hold");
        let v = call(
            &mut server,
            json!({"name": "explain_exit_code", "arguments": {"code": 5}}),
        );
        assert_eq!(v["result"]["isError"], true);
        let v = call(
            &mut server,
            json!({"name": "explain_exit_code", "arguments": {"code": "7"}}),
        );
        assert_eq!(v["result"]["isError"], true);
        let v = call(
            &mut server,
            json!({"name": "exit_code_registry", "arguments": {}}),
        );
        assert_eq!(
            v["result"]["structuredContent"]["codes"]
                .as_array()
                .unwrap()
                .len(),
            EXIT_CODES.len()
        );
        assert_eq!(
            call(&mut server, json!({"name": "nope"}))["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            call(&mut server, json!({"arguments": {}}))["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            call(
                &mut server,
                json!({"name": "review_patch", "arguments": []})
            )["error"]["code"],
            INVALID_PARAMS
        );
    }

    #[test]
    fn only_a_jsonrpc_2_envelope_is_acted_on() {
        let mut server = Server::default();
        for bad in [
            r#"{"jsonrpc":"1.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
            r#"{"id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
            r#"{"jsonrpc":2,"id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
        ] {
            let v = server.handle_line(bad).unwrap();
            assert_eq!(
                (v["error"]["code"].as_i64(), v["id"].as_i64()),
                (Some(-32600), Some(1)),
                "{bad}"
            );
        }
        // None of them established the legacy handshake.
        let v = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        assert_eq!(v["error"]["code"], INVALID_PARAMS);
        assert_eq!(
            server.handle_line(r#"{"jsonrpc":"1.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(
            server
                .handle_line(r#"{"jsonrpc":"1.0","id":null,"method":"ping"}"#)
                .unwrap()["error"]["code"],
            -32600
        );
    }

    #[test]
    fn malformed_messages_and_notifications() {
        let mut server = Server::default();
        assert_eq!(
            server.handle_line("{").unwrap()["error"]["code"],
            PARSE_ERROR
        );
        assert_eq!(server.handle_line("[]").unwrap()["error"]["code"], -32600);
        assert_eq!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#)
                .unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            server.handle_line(r#"{"jsonrpc":"2.0","id":1}"#).unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[]}"#)
                .unwrap()["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            server.handle_line(
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#
            ),
            None
        );
        for id in [json!("s"), json!(-3), json!(u64::MAX)] {
            let line = json!({"jsonrpc": "2.0", "id": id, "method": "ping", "params": {"_meta": {PROTOCOL_VERSION_KEY: MODERN, CLIENT_CAPABILITIES_KEY: {}}}}).to_string();
            assert_eq!(server.handle_line(&line).unwrap()["id"], id);
        }
        assert_eq!(
            server
                .handle_line(r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#)
                .unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            server.handle_line(&modern("no/such", &json!({}))).unwrap()["error"]["code"],
            METHOD_NOT_FOUND
        );
        assert_eq!(
            server.handle_line(&modern("ping", &json!({}))).unwrap()["result"]["resultType"],
            "complete"
        );
    }

    #[test]
    fn serve_writes_one_line_per_request_and_nothing_for_notifications() {
        let input = format!(
            "{}\n\n{}\n",
            modern("ping", &json!({})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        );
        let text = within(5, move || {
            let mut out = Vec::new();
            serve(&mut input.as_bytes(), &mut out).unwrap();
            String::from_utf8(out).unwrap()
        });
        assert_eq!(text.lines().count(), 1, "{text}");
        assert_eq!(serde_json::from_str::<Value>(text.trim()).unwrap()["id"], 1);
    }
}
