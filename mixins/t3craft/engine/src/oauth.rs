//! The T3 Code sign-in, a port of `t3/OAuthLogin.kt`: dynamic client
//! registration, a PKCE authorization URL the player approves in a browser,
//! a one-shot callback server on 127.0.0.1, and the code-for-token exchange.

use crate::config::{now_millis, StoredCredential};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const CALLBACK_PATH: &str = "/callback";
const SCOPES: &str = "orchestration:read orchestration:operate";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A sign-in in flight: `authorize_url` is where the player goes,
/// `result` delivers the credential (or why it failed), `cancel` abandons it.
pub struct PendingLogin {
    pub authorize_url: String,
    pub result: mpsc::Receiver<Result<StoredCredential, String>>,
    cancel: Arc<AtomicBool>,
}

impl PendingLogin {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Register and wait for the player: bind the callback first, as the Java mod
/// does, so the URL it opens already answers.
pub fn start(server_url: &str) -> Result<PendingLogin, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Could not start the sign-in listener: {e}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Could not start the sign-in listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Could not start the sign-in listener: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");

    let client_id = register(server_url, &redirect_uri)?;
    let verifier = random_url_safe(48);
    let state = random_url_safe(24);
    let authorize_url = authorize_url(server_url, &client_id, &redirect_uri, &verifier, &state);

    let (tx, result) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let server_url = server_url.to_owned();
        std::thread::Builder::new()
            .name("t3craft-oauth".into())
            .spawn(move || {
                let outcome = wait_for_code(&listener, &state, &cancel)
                    .and_then(|code| {
                        exchange_code(&server_url, &client_id, &redirect_uri, &verifier, &code)
                    });
                let _ = tx.send(outcome);
            })
            .map_err(|e| format!("Could not start the sign-in thread: {e}"))?;
    }

    Ok(PendingLogin {
        authorize_url,
        result,
        cancel,
    })
}

fn register(server_url: &str, redirect_uri: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "client_name": "T3 Craft (Horizon)",
        "redirect_uris": [redirect_uri],
    });
    let json = post_json(&format!("{server_url}/oauth/mcp/register"), &body, server_url)?;
    json.get("client_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            json.get("error_description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("T3 Code refused to register the mod.")
                .to_owned()
        })
}

fn authorize_url(
    server_url: &str,
    client_id: &str,
    redirect_uri: &str,
    verifier: &str,
    state: &str,
) -> String {
    let query = form_encode(&[
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("code_challenge", &s256(verifier)),
        ("code_challenge_method", "S256"),
        ("scope", SCOPES),
        ("resource", &format!("{server_url}/mcp")),
        ("state", state),
    ]);
    format!("{server_url}/oauth/mcp/authorize?{query}")
}

/// One browser hit, or give up: answer with the page the Java mod shows.
fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled.".to_owned());
        }
        if Instant::now() > deadline {
            return Err("Sign-in expired, sign in again".to_owned());
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Browsers open idle preconnects: without a deadline a silent
                // socket would hold the single accept loop while the real
                // /callback request waits in the backlog.
                let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(8)));
                match handle_callback(stream, expected_state) {
                    CallbackOutcome::Code(code) => return Ok(code),
                    CallbackOutcome::Failed(message) => return Err(message),
                    CallbackOutcome::KeepWaiting => {}
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("The sign-in listener failed: {e}")),
        }
    }
}

enum CallbackOutcome {
    Code(String),
    Failed(String),
    KeepWaiting,
}

/// Read the `GET /callback?…`, answer its little page, say how it went.
fn handle_callback(stream: std::net::TcpStream, expected_state: &str) -> CallbackOutcome {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return CallbackOutcome::KeepWaiting;
    }
    // Drain the rest of the head so the response isn't a surprise.
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok() && line.trim() != "" {}

    let path = request_line.split_whitespace().nth(1).unwrap_or_default();
    let params: std::collections::HashMap<String, String> = url::Url::parse(
        &format!("http://localhost{path}"),
    )
    .map(|u| u.query_pairs().into_owned().collect())
    .unwrap_or_default();

    let (message, outcome) = callback_outcome(&params, expected_state);
    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>T3 Craft</title></head>\
         <body style=\"font-family:system-ui;background:#111;color:#eee;display:grid;place-items:center;height:100vh;margin:0\">\
         <p style=\"font-size:1.2rem\">{}</p></body></html>",
        escape_html(&message)
    );
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        page.len(),
        page
    );
    outcome
}

fn callback_outcome(
    params: &std::collections::HashMap<String, String>,
    expected_state: &str,
) -> (String, CallbackOutcome) {
    let get = |k: &str| params.get(k).map(String::as_str);
    match (get("state"), get("error"), get("code")) {
        // A missing or wrong state is stale, as the Java mod checks first.
        (state, _, _) if state != Some(expected_state) => (
            "This sign-in link is stale. Start again from the game.".into(),
            CallbackOutcome::Failed("This sign-in link is stale. Start again.".into()),
        ),
        (_, Some("access_denied"), _) => (
            "You declined the sign-in. The game stays disconnected.".into(),
            CallbackOutcome::Failed("Sign-in declined.".into()),
        ),
        (_, Some(error), _) => {
            let detail = get("error_description").unwrap_or(error);
            (
                format!("Sign-in failed: {detail}"),
                CallbackOutcome::Failed(format!("Sign-in failed: {detail}")),
            )
        }
        (_, _, None) => (
            "T3 Code sent no authorization code.".into(),
            CallbackOutcome::Failed("T3 Code sent no authorization code.".into()),
        ),
        (_, _, Some(code)) => (
            "T3 Craft is connected. You can close this tab and go back to the game.".into(),
            CallbackOutcome::Code(code.to_owned()),
        ),
    }
}

fn exchange_code(
    server_url: &str,
    client_id: &str,
    redirect_uri: &str,
    verifier: &str,
    code: &str,
) -> Result<StoredCredential, String> {
    let body = form_encode(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
        ("resource", &format!("{server_url}/mcp")),
    ]);
    let json = post_form(&format!("{server_url}/oauth/mcp/token"), &body, server_url)?;
    let token = json
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            json.get("error_description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("T3 Code did not issue a token.")
                .to_owned()
        })?;
    let lifetime = json
        .get("expires_in")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(30 * 24 * 3600);
    Ok(StoredCredential {
        server_url: server_url.to_owned(),
        access_token: token,
        expires_at_epoch_millis: now_millis() + lifetime * 1000,
    })
}

fn post_json(url: &str, body: &serde_json::Value, server_url: &str) -> Result<serde_json::Value, String> {
    send_or_explain(
        ureq::post(url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string()),
        url,
        server_url,
    )
}

fn post_form(url: &str, body: &str, server_url: &str) -> Result<serde_json::Value, String> {
    send_or_explain(
        ureq::post(url)
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(body),
        url,
        server_url,
    )
}

fn send_or_explain(
    result: Result<ureq::Response, ureq::Error>,
    url: &str,
    server_url: &str,
) -> Result<serde_json::Value, String> {
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            return serde_json::from_str(&text).map_err(|_| {
                format!("T3 Code answered HTTP {code} without JSON at {url}.")
            });
        }
        Err(ureq::Error::Transport(_)) => {
            return Err(format!("Could not reach T3 Code at {server_url}. Is it running?"));
        }
    };
    let text = response.into_string().unwrap_or_default();
    serde_json::from_str(&text)
        .map_err(|_| format!("T3 Code answered without JSON at {url}."))
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().map(|(k, v)| (*k, *v)))
        .finish()
}

fn s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()))
}

fn random_url_safe(byte_count: usize) -> String {
    let mut bytes = vec![0u8; byte_count];
    let _ = getrandom::getrandom(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_challenge_is_s256() {
        // RFC 7636's example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(s256(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn callback_rejects_a_wrong_state() {
        let params: std::collections::HashMap<String, String> =
            [("state".to_owned(), "other".to_owned())].into_iter().collect();
        let (_, outcome) = callback_outcome(&params, "mine");
        assert!(matches!(outcome, CallbackOutcome::Failed(_)));
    }

    #[test]
    fn callback_takes_a_good_code() {
        let params: std::collections::HashMap<String, String> = [
            ("state".to_owned(), "mine".to_owned()),
            ("code".to_owned(), "abc".to_owned()),
        ]
        .into_iter()
        .collect();
        let (_, outcome) = callback_outcome(&params, "mine");
        assert!(matches!(outcome, CallbackOutcome::Code(c) if c == "abc"));
    }

    #[test]
    fn callback_explains_a_denial() {
        let params: std::collections::HashMap<String, String> = [
            ("state".to_owned(), "mine".to_owned()),
            ("error".to_owned(), "access_denied".to_owned()),
        ]
        .into_iter()
        .collect();
        let (_, outcome) = callback_outcome(&params, "mine");
        assert!(matches!(outcome, CallbackOutcome::Failed(_)));
    }
}
