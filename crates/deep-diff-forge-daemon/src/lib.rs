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
//! validated, the socket is `0600`, and stale sockets are replaced on bind.

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
    runtime_dir, validate_private_dir,
};
pub use serve::{bind_secure, handle_connection, process_line, request, run_server};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_default_socket_path() {
        assert!(
            default_socket_path()
                .to_string_lossy()
                .contains("deep-diff-forge")
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
