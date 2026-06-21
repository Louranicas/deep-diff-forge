//! Optional UDS JSON-RPC review daemon for Deep-Diff-Forge (L7).
//!
//! The daemon accelerates repeated review and multi-client workflows; it is
//! never required for one-shot CLI correctness. It is built std-first
//! (`std::os::unix::net`, thread-per-connection) — no async runtime — with
//! `serde_json` for JSON-RPC 2.0 framing. The protocol, dispatch, and socket
//! security are unit-tested; the real socket round-trip is exercised in-process
//! (`UnixStream::pair` and a live [`run_server`] thread), leaving no
//! meaningful logic untested.
//!
//! Security: the runtime directory is created owner-private (`0700`) and
//! validated (symlinks rejected; `chmod` doubles as an ownership gate), the
//! socket is `0600`, stale sockets are replaced on bind, each request line is
//! size-bounded, connections carry a read timeout, and a panic in dispatch is
//! contained so one abusive request cannot tear down the daemon. There is no
//! world-writable `/tmp` fallback: without `$XDG_RUNTIME_DIR` the daemon fails
//! closed and the operator passes `--socket PATH`.

mod handler;
mod protocol;
mod security;
mod serve;

pub use handler::{Engine, dispatch};
pub use protocol::{
    INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR,
    PATCH_PARSE_FAILED, PROTOCOL_VERSION, Request, RpcError, SESSION_NOT_FOUND, error_response,
    parse_request, success_response,
};
pub use security::{
    SECURE_DIR_MODE, SECURE_SOCKET_MODE, SocketError, default_socket_path, ensure_runtime_dir,
    runtime_base, runtime_base_from, runtime_dir, validate_private_dir,
};
pub use serve::{bind_secure, handle_connection, process_line, request, run_server};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_default_socket_path() {
        // Pure-resolver chain: a present XDG base yields a deep-diff-forge path;
        // an absent one yields None (fail closed, no /tmp fallback).
        let base = runtime_base_from(Some(std::ffi::OsString::from("/run/user/1000")));
        let sock = base.map(|b| b.join("deep-diff-forge").join("deep-diff-forge.sock"));
        assert!(sock.unwrap().to_string_lossy().contains("deep-diff-forge"));
        assert_eq!(
            runtime_base_from(None).map(|b| b.join("deep-diff-forge.sock")),
            None
        );
    }

    #[test]
    fn facade_process_line_works() {
        let engine = std::sync::Mutex::new(Engine::new());
        let resp = process_line(r#"{"id":1,"method":"daemon.health"}"#, &engine);
        assert!(resp.contains("\"status\":\"ok\""));
    }

    #[test]
    fn facade_dispatch_works() {
        let mut engine = Engine::new();
        let req = parse_request(r#"{"method":"engine.initialize"}"#).unwrap();
        let result = dispatch(&req, &mut engine).unwrap();
        assert_eq!(result["server"], "deep-diff-forge");
    }
}
