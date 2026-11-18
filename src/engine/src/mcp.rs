//! The MCP-over-HTTP client the Java mod's `McpClient.kt` is: JSON-RPC posts to
//! `{server}/mcp`, a session id in `Mcp-Session-Id`, and tool results as MCP
//! `structuredContent`. Blocking (`ureq`): the monitor runs it on its own
//! thread, off the game's.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SESSION_HEADER: &str = "Mcp-Session-Id";

/// The token T3 Code saved was refused (401/403): sign in again.
#[derive(Debug)]
pub struct Unauthorized;

/// The tool call itself failed (the MCP `isError` result).
#[derive(Debug)]
pub struct ToolError(pub String);

/// Anything else: T3 Code unreachable, a bad reply, a non-tool MCP error.
#[derive(Debug)]
pub struct ProtocolError(pub String);

#[derive(Debug)]
pub enum McpError {
    Unauthorized,
    Tool(String),
    Protocol(String),
}

impl From<Unauthorized> for McpError {
    fn from(_: Unauthorized) -> Self {
        McpError::Unauthorized
    }
}
impl From<ToolError> for McpError {
    fn from(e: ToolError) -> Self {
        McpError::Tool(e.0)
    }
}
impl From<ProtocolError> for McpError {
    fn from(e: ProtocolError) -> Self {
        McpError::Protocol(e.0)
    }
}

pub struct McpClient {
    agent: ureq::Agent,
    endpoint: String,
    access_token: String,
    client_version: String,
    next_id: AtomicU64,
    session_id: Option<String>,
}

impl McpClient {
    pub fn new(server_url: &str, access_token: &str, client_version: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .build();
        Self {
            agent,
            endpoint: format!("{server_url}/mcp"),
            access_token: access_token.to_owned(),
            client_version: client_version.to_owned(),
            next_id: AtomicU64::new(1),
            session_id: None,
        }
    }

    /// `tools/call`, answering the call's `structuredContent`.
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = self.request_with_session("tools/call", params)?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(ToolError(tool_error_text(&result)).into());
        }
        result
            .get("structuredContent")
            .cloned()
            .ok_or_else(|| ProtocolError(format!("{name} returned no structured content.")).into())
    }

    /// Leave the session, as the Java mod's `close()`.
    pub fn close(&mut self) {
        let Some(id) = self.session_id.take() else {
            return;
        };
        let _ = self
            .agent
            .delete(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.access_token))
            .set(SESSION_HEADER, &id)
            .call();
    }

    fn request_with_session(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, McpError> {
        if self.session_id.is_none() {
            self.initialize()?;
        }
        match self.request(method, params.clone()) {
            Err(McpError::Protocol(e)) if e == SESSION_EXPIRED => {
                self.initialize()?;
                self.request(method, params)
            }
            other => other,
        }
    }

    fn initialize(&mut self) -> Result<Value, McpError> {
        self.session_id = None;
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "t3craft", "version": self.client_version },
        });
        let result = self.request("initialize", params)?;
        self.notify("notifications/initialized");
        Ok(result)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let (session, content_type, text) = self.post(&body)?;
        if let Some(session) = session {
            self.session_id = Some(session);
        }
        let message = find_response(&content_type, &text, id)?;
        if let Some(error) = message.get("error") {
            let msg = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("MCP error.")
                .to_owned();
            return Err(ProtocolError(format!("{msg} (on {method})")).into());
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str) {
        let body = json!({ "jsonrpc": "2.0", "method": method });
        let _ = self.post(&body);
    }

    /// POST one message; the reply's session header, content type and body.
    fn post(&mut self, body: &Value) -> Result<(Option<String>, String, String), McpError> {
        let mut request = self
            .agent
            .post(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.access_token))
            .set("Accept", "application/json, text/event-stream")
            .set("MCP-Protocol-Version", PROTOCOL_VERSION)
            .set("Content-Type", "application/json");
        if let Some(id) = &self.session_id {
            request = request.set(SESSION_HEADER, id);
        }
        match request.send_string(&body.to_string()) {
            Ok(response) => {
                let session = response.header(SESSION_HEADER).map(str::to_owned);
                let content_type = response.content_type().to_owned();
                let text = response
                    .into_string()
                    .map_err(|e| ProtocolError(format!("T3 Code's reply was not UTF-8: {e}")))?;
                Ok((session, content_type, text))
            }
            Err(ureq::Error::Status(code, response)) => match code {
                401 | 403 => Err(Unauthorized.into()),
                404 if self.session_id.is_some() => {
                    Err(ProtocolError(SESSION_EXPIRED.to_owned()).into())
                }
                _ => {
                    let text = response.into_string().unwrap_or_default();
                    Err(ProtocolError(format!(
                        "T3 Code answered HTTP {code}: {}",
                        text.chars().take(200).collect::<String>()
                    ))
                    .into())
                }
            },
            Err(ureq::Error::Transport(e)) => Err(ProtocolError(format!(
                "Could not reach T3 Code: {e}"
            ))
            .into()),
        }
    }
}

const SESSION_EXPIRED: &str = "__session_expired__";

/// The JSON-RPC message answering `id` in a JSON or SSE body.
fn find_response(content_type: &str, body: &str, id: u64) -> Result<Value, McpError> {
    let mut messages = Vec::new();
    if content_type.starts_with("text/event-stream") {
        for payload in sse_data_payloads(body) {
            if let Ok(v) = serde_json::from_str::<Value>(&payload) {
                messages.extend(flatten_batch(v));
            }
        }
    } else if let Ok(v) = serde_json::from_str::<Value>(body) {
        messages.extend(flatten_batch(v));
    }
    messages
        .into_iter()
        .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
        .ok_or_else(|| ProtocolError(format!("T3 Code sent no reply to request {id}.")).into())
}

fn flatten_batch(element: Value) -> Vec<Value> {
    match element {
        Value::Array(items) => items,
        other => vec![other],
    }
}

/// `data:` payloads of an SSE stream, one per event, blank lines splitting.
fn sse_data_payloads(body: &str) -> Vec<String> {
    body.split("\n\n")
        .map(|event| {
            event
                .lines()
                .filter(|line| line.starts_with("data:"))
                .map(|line| line.trim_start_matches("data:").trim_start().to_owned())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|event| !event.is_empty())
        .collect()
}

/// The message of an MCP error result: `content[0].text`, which T3 Code
/// sometimes fills with a JSON `{"message": …}` of its own.
fn tool_error_text(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content
                .iter()
                .find_map(|item| item.get("text").and_then(Value::as_str))
        })
        .unwrap_or_default();
    let structured = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("message")?.as_str().map(str::to_owned));
    structured.unwrap_or_else(|| {
        if text.is_empty() {
            "The T3 Code tool call failed.".to_owned()
        } else {
            text.to_owned()
        }
    })
}

/// What the API and the UI say for one error.
pub fn describe(error: &McpError) -> String {
    match error {
        McpError::Unauthorized => "T3 Code rejected the saved sign-in.".to_owned(),
        McpError::Tool(message) | McpError::Protocol(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_sse_reply() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let message = find_response("text/event-stream", body, 7).unwrap();
        assert_eq!(message["result"]["ok"], true);
    }

    #[test]
    fn parses_a_json_batch() {
        let body = r#"[{"jsonrpc":"2.0","id":1,"result":{}},{"jsonrpc":"2.0","id":2,"result":{"a":1}}]"#;
        let message = find_response("application/json", body, 2).unwrap();
        assert_eq!(message["result"]["a"], 1);
    }

    #[test]
    fn unfolds_an_error_text() {
        let result = json!({
            "isError": true,
            "content": [{"type": "text", "text": "{\"message\":\"thread gone\"}"}],
        });
        assert_eq!(tool_error_text(&result), "thread gone");
    }
}
