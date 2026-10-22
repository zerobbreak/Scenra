//! Minimal Chrome DevTools Protocol client: the `/json` HTTP endpoints plus a websocket
//! session per target. Just enough for Phase 0. Swap it for a full client if it grows.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const TIMEOUT: Duration = Duration::from_secs(5);

/// `GET /json/version`.
#[derive(Deserialize)]
pub struct Version {
    /// e.g. `Edg/141.0.3537.71` for WebView2 and Edge.
    #[serde(rename = "Browser")]
    pub browser: String,
    #[serde(rename = "User-Agent")]
    pub user_agent: String,
}

/// One entry of `GET /json/list`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub web_socket_debugger_url: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn version(port: u16) -> Result<Version, String> {
    get_json(port, "/json/version")
}

pub fn targets(port: u16) -> Result<Vec<Target>, String> {
    get_json(port, "/json/list")
}

/// One-shot HTTP/1.1 GET against the loopback debug port. DevTools sends a
/// `Content-Length`, so we stop reading there and don't wait for the socket to close.
fn get_json<T: DeserializeOwned>(port: u16, path: &str) -> Result<T, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(err)?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(err)?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
        .map_err(err)?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let body = loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = std::str::from_utf8(&buf[..end]).map_err(err)?;
            let status = head.lines().next().unwrap_or_default();
            if !status.contains(" 200 ") {
                return Err(format!("{path}: {status}"));
            }
            let length = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
            });
            if let Some(length) = length
                && buf.len() >= end + 4 + length
            {
                break &buf[end + 4..end + 4 + length];
            }
        }
        let n = stream.read(&mut chunk).map_err(err)?;
        if n == 0 {
            let end = buf.windows(4).position(|w| w == b"\r\n\r\n").ok_or("truncated response")?;
            break &buf[end + 4..];
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    serde_json::from_slice(body).map_err(|e| format!("{path}: {e}"))
}

/// A websocket session with one target (page or browser).
pub struct Session {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Session {
    pub fn connect(ws_url: &str) -> Result<Self, String> {
        let (socket, _) = tungstenite::connect(ws_url).map_err(err)?;
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream.set_read_timeout(Some(TIMEOUT)).map_err(err)?;
        }
        Ok(Self { socket, next_id: 0 })
    }

    /// Sends a command and waits for its reply. Events received meanwhile are dropped.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "id": id, "method": method, "params": params });
        self.socket.send(Message::text(request.to_string())).map_err(err)?;
        loop {
            let Message::Text(text) = self.socket.read().map_err(err)? else { continue };
            let mut reply: Value = serde_json::from_str(&text).map_err(err)?;
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(format!("{method}: {}", error["message"]));
            }
            return Ok(reply["result"].take());
        }
    }

    /// Evaluates a JS expression in the page and returns its value as JSON.
    pub fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let mut result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )?;
        if let Some(details) = result.get("exceptionDetails") {
            let message = &details["exception"]["description"];
            return Err(format!("evaluate failed: {}", message.as_str().unwrap_or("unknown error")));
        }
        Ok(result["result"]["value"].take())
    }
}
