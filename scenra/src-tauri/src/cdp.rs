//! Minimal Chrome DevTools Protocol client: the `/json` HTTP endpoints plus a websocket
//! session per target. Just enough for Phase 0. Swap it for a full client if it grows.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const TIMEOUT: Duration = Duration::from_secs(5);
/// Events kept while a call waits for its reply. Sessions that never read events keep only the latest.
const MAX_QUEUED_EVENTS: usize = 256;

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
    /// Events that arrived while a call waited for its reply, oldest first.
    events: VecDeque<Value>,
}

impl Session {
    pub fn connect(ws_url: &str) -> Result<Self, String> {
        // Time out the handshake too, so an endpoint that accepts and never answers can't hang
        // the caller.
        let host = ws_url.strip_prefix("ws://").and_then(|rest| rest.split('/').next()).ok_or("not a ws:// DevTools URL")?;
        let addr = std::net::ToSocketAddrs::to_socket_addrs(host).map_err(err)?.next().ok_or("DevTools URL has no address")?;
        let stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(err)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(err)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(err)?;
        let (socket, _) = tungstenite::client(ws_url, MaybeTlsStream::Plain(stream)).map_err(err)?;
        Ok(Self { socket, next_id: 0, events: VecDeque::new() })
    }

    /// Sends a command and waits for its reply. Events received meanwhile are queued for
    /// [`Session::next_event`].
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "id": id, "method": method, "params": params });
        self.socket.send(Message::text(request.to_string())).map_err(err)?;
        loop {
            let Message::Text(text) = self.socket.read().map_err(err)? else { continue };
            let mut reply: Value = serde_json::from_str(&text).map_err(err)?;
            if reply.get("method").is_some() && reply.get("id").is_none() {
                self.queue(reply);
                continue;
            }
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

    /// The next event (`{method, params}`), waiting up to `timeout`; `None` if none arrived.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Value>, String> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let MaybeTlsStream::Plain(stream) = self.socket.get_ref() else {
            return Err("CDP events need a plain loopback connection".into());
        };
        // A zero timeout would mean "block forever".
        stream.set_read_timeout(Some(timeout.max(Duration::from_millis(1)))).map_err(err)?;
        let read = loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let message: Value = serde_json::from_str(&text).map_err(err)?;
                    if message.get("method").is_some() && message.get("id").is_none() {
                        break Ok(Some(message));
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    break Ok(None);
                }
                Err(e) => break Err(err(e)),
            }
        };
        if let MaybeTlsStream::Plain(stream) = self.socket.get_ref() {
            stream.set_read_timeout(Some(TIMEOUT)).map_err(err)?;
        }
        read
    }

    fn queue(&mut self, event: Value) {
        if self.events.len() == MAX_QUEUED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    /// A one-connection DevTools stand-in: sends an event, then answers the first call with
    /// another event ahead of the reply.
    fn fake_devtools() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!("ws://{}/devtools/page/test", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let send = |socket: &mut WebSocket<TcpStream>, value: Value| socket.send(Message::text(value.to_string())).unwrap();
            send(&mut socket, json!({ "method": "Runtime.bindingCalled", "params": { "payload": "first" } }));
            let Message::Text(request) = socket.read().unwrap() else { panic!("expected a call") };
            let id = serde_json::from_str::<Value>(&request).unwrap()["id"].clone();
            send(&mut socket, json!({ "method": "Runtime.bindingCalled", "params": { "payload": "second" } }));
            send(&mut socket, json!({ "id": id, "result": { "ok": true } }));
            // Keep the connection open until the client hangs up.
            while socket.read().is_ok() {}
        });
        (url, server)
    }

    #[test]
    fn events_received_during_a_call_are_kept_in_order() {
        let (url, server) = fake_devtools();
        let mut session = Session::connect(&url).unwrap();
        assert_eq!(session.call("Runtime.addBinding", json!({})).unwrap()["ok"], json!(true));
        let payloads: Vec<_> = (0..2)
            .map(|_| session.next_event(Duration::from_secs(2)).unwrap().unwrap()["params"]["payload"].clone())
            .collect();
        assert_eq!(payloads, [json!("first"), json!("second")]);
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn connect_gives_up_on_an_endpoint_that_never_answers_the_handshake() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let url = format!("ws://{}/devtools/page/gone", listener.local_addr().unwrap());
        // Accept, then say nothing.
        let silent = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
        let started = std::time::Instant::now();
        assert!(Session::connect(&url).is_err());
        assert!(started.elapsed() < TIMEOUT + Duration::from_secs(2));
        drop(silent.join());
    }

    #[test]
    fn next_event_times_out_without_an_event() {
        let (url, server) = fake_devtools();
        let mut session = Session::connect(&url).unwrap();
        assert!(session.next_event(Duration::from_secs(2)).unwrap().is_some());
        let started = std::time::Instant::now();
        assert!(session.next_event(Duration::from_millis(150)).unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
        // The session still works for calls after a timed-out read.
        assert_eq!(session.call("Runtime.addBinding", json!({})).unwrap()["ok"], json!(true));
        drop(session);
        server.join().unwrap();
    }
}
