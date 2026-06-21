use serde::Deserialize;
use serde_json::{Value, json};

/// Engine protocol version advertised by `engine.initialize`/`daemon.health`.
pub const PROTOCOL_VERSION: u32 = 0;

/// JSON-RPC standard error code: invalid JSON was received.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC standard error code: the request object is invalid.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC standard error code: the method does not exist.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC standard error code: invalid method parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC standard error code: internal error.
pub const INTERNAL_ERROR: i64 = -32603;
/// Domain error code: the requested session does not exist.
pub const SESSION_NOT_FOUND: i64 = 1;
/// Domain error code: a supplied patch could not be parsed.
pub const PATCH_PARSE_FAILED: i64 = 4;

/// A parsed JSON-RPC request. Missing optional fields default rather than
/// failing, so a terse client (`{"method":"daemon.health"}`) is accepted.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Protocol marker (`"2.0"`); defaulted when absent.
    #[serde(default)]
    pub jsonrpc: String,
    /// Correlation id, echoed in the response; `null` when absent.
    #[serde(default)]
    pub id: Value,
    /// Method name.
    pub method: String,
    /// Method parameters; `null` when absent.
    #[serde(default)]
    pub params: Value,
}

/// A structured RPC error (code + message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
}

impl RpcError {
    /// Construct an error.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Method-not-found error for `method`.
    #[must_use]
    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("method not found: {method}"))
    }

    /// Invalid-params error with a reason.
    #[must_use]
    pub fn invalid_params(reason: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, reason.into())
    }
}

/// Parse one JSON-RPC request line.
///
/// # Errors
///
/// Returns a [`RpcError`] with [`PARSE_ERROR`] when the line is not a valid
/// JSON-RPC request object.
pub fn parse_request(line: &str) -> Result<Request, RpcError> {
    serde_json::from_str::<Request>(line).map_err(|e| RpcError::new(PARSE_ERROR, e.to_string()))
}

/// Serialize a success response for `id` with `result`.
#[must_use]
pub fn success_response(id: &Value, result: Value) -> String {
    let mut object = serde_json::Map::new();
    object.insert("jsonrpc".to_string(), Value::from("2.0"));
    object.insert("id".to_string(), id.clone());
    object.insert("result".to_string(), result);
    Value::Object(object).to_string()
}

/// Serialize an error response for `id`.
#[must_use]
pub fn error_response(id: &Value, error: &RpcError) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": error.code, "message": error.message}
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_request() {
        let req = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"daemon.health","params":{}}"#)
            .unwrap();
        assert_eq!(req.method, "daemon.health");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, json!(1));
    }

    #[test]
    fn parses_terse_request_with_defaults() {
        let req = parse_request(r#"{"method":"daemon.status"}"#).unwrap();
        assert_eq!(req.method, "daemon.status");
        assert_eq!(req.id, Value::Null);
        assert_eq!(req.params, Value::Null);
    }

    #[test]
    fn missing_method_is_parse_error() {
        let err = parse_request(r#"{"id":1}"#).unwrap_err();
        assert_eq!(err.code, PARSE_ERROR);
    }

    #[test]
    fn invalid_json_is_parse_error() {
        let err = parse_request("not json").unwrap_err();
        assert_eq!(err.code, PARSE_ERROR);
    }

    #[test]
    fn string_id_is_preserved() {
        let req = parse_request(r#"{"id":"abc","method":"m"}"#).unwrap();
        assert_eq!(req.id, json!("abc"));
    }

    #[test]
    fn params_object_is_preserved() {
        let req = parse_request(r#"{"method":"diff.plan","params":{"patch":"x"}}"#).unwrap();
        assert_eq!(req.params.get("patch").and_then(Value::as_str), Some("x"));
    }

    #[test]
    fn success_response_shape() {
        let s = success_response(&json!(7), json!({"ok": true}));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"]["ok"], true);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn error_response_shape() {
        let s = error_response(&json!(1), &RpcError::method_not_found("foo"));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["error"]["code"], METHOD_NOT_FOUND);
        assert!(v["error"]["message"].as_str().unwrap().contains("foo"));
        assert!(v.get("result").is_none());
    }

    #[test]
    fn error_response_preserves_null_id() {
        let s = error_response(&Value::Null, &RpcError::new(PARSE_ERROR, "bad"));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["id"], Value::Null);
    }

    #[test]
    fn rpc_error_constructors() {
        assert_eq!(RpcError::method_not_found("m").code, METHOD_NOT_FOUND);
        assert_eq!(RpcError::invalid_params("why").code, INVALID_PARAMS);
        assert_eq!(
            RpcError::new(SESSION_NOT_FOUND, "x").code,
            SESSION_NOT_FOUND
        );
    }

    #[test]
    fn protocol_version_is_zero() {
        assert_eq!(PROTOCOL_VERSION, 0);
    }

    #[test]
    fn response_round_trips_through_serde() {
        let s = success_response(&json!("id-1"), json!([1, 2, 3]));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["result"], json!([1, 2, 3]));
        assert_eq!(v["id"], "id-1");
    }
}
