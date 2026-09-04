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
/// Domain error code: a bounded daemon resource budget would be exceeded.
pub const RESOURCE_EXHAUSTED: i64 = 8;

/// A parsed JSON-RPC request. Missing optional fields default rather than
/// failing, so a terse client (`{"method":"daemon.health"}`) is accepted.
#[derive(Debug, Clone)]
pub struct Request {
    /// Protocol marker (`"2.0"`); defaulted when absent.
    pub jsonrpc: String,
    /// Correlation id, echoed in the response; `null` when absent.
    pub id: Value,
    /// Method name.
    pub method: String,
    /// Method parameters; `null` when absent.
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
/// Invalid JSON is a [`PARSE_ERROR`]; valid JSON with an invalid JSON-RPC shape
/// is an [`INVALID_REQUEST`]. The protocol marker may be omitted for backwards
/// compatibility with the CLI's terse local requests, but when present it must
/// be exactly `"2.0"`.
pub fn parse_request(line: &str) -> Result<Request, RpcError> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| RpcError::new(PARSE_ERROR, e.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| RpcError::new(INVALID_REQUEST, "request must be a JSON object"))?;

    let jsonrpc = match object.get("jsonrpc") {
        None => String::new(),
        Some(Value::String(version)) if version == "2.0" => version.clone(),
        Some(_) => {
            return Err(RpcError::new(
                INVALID_REQUEST,
                "jsonrpc must be exactly \"2.0\"",
            ));
        }
    };
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty() && method.len() <= 256)
        .ok_or_else(|| {
            RpcError::new(
                INVALID_REQUEST,
                "method must be a non-empty string of at most 256 bytes",
            )
        })?
        .to_string();
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    if !matches!(id, Value::Null | Value::String(_) | Value::Number(_)) {
        return Err(RpcError::new(
            INVALID_REQUEST,
            "id must be a string, number, or null",
        ));
    }
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    if !matches!(params, Value::Null | Value::Object(_) | Value::Array(_)) {
        return Err(RpcError::new(
            INVALID_REQUEST,
            "params must be an object, array, or null",
        ));
    }

    Ok(Request {
        jsonrpc,
        id,
        method,
        params,
    })
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
    fn missing_method_is_invalid_request() {
        let err = parse_request(r#"{"id":1}"#).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
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
    fn rejects_wrong_protocol_version() {
        let err = parse_request(r#"{"jsonrpc":"1.0","method":"m"}"#).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
    }

    #[test]
    fn rejects_non_object_request() {
        let err = parse_request(r"[1,2,3]").unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
    }

    #[test]
    fn rejects_structured_id() {
        let err = parse_request(r#"{"id":{},"method":"m"}"#).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
    }

    #[test]
    fn rejects_scalar_params() {
        let err = parse_request(r#"{"method":"m","params":true}"#).unwrap_err();
        assert_eq!(err.code, INVALID_REQUEST);
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
    fn json_rpc_error_codes_match_wire_standard() {
        // These are externally-observed wire protocol values, not arbitrary
        // internal enum discriminants. A sign flip silently changes client
        // compatibility and must be killed by tests.
        assert_eq!(PARSE_ERROR, -32700);
        assert_eq!(INVALID_REQUEST, -32600);
        assert_eq!(METHOD_NOT_FOUND, -32601);
        assert_eq!(INVALID_PARAMS, -32602);
        assert_eq!(INTERNAL_ERROR, -32603);
        assert_eq!(SESSION_NOT_FOUND, 1);
        assert_eq!(PATCH_PARSE_FAILED, 4);
        assert_eq!(RESOURCE_EXHAUSTED, 8);
    }

    #[test]
    fn response_round_trips_through_serde() {
        let s = success_response(&json!("id-1"), json!([1, 2, 3]));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["result"], json!([1, 2, 3]));
        assert_eq!(v["id"], "id-1");
    }
}
