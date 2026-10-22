//! ObsClient (PROJECT_OUTLINE §4): a small obs-websocket v5 client over a sync websocket.

use crate::secrets;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::TcpStream;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub const DEFAULT_URL: &str = "ws://127.0.0.1:4455";

pub struct Obs {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// obs-websocket v5 auth: base64(sha256(base64(sha256(password + salt)) + challenge)).
fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret = B64.encode(Sha256::digest(format!("{password}{salt}")));
    B64.encode(Sha256::digest(format!("{secret}{challenge}")))
}

impl Obs {
    pub fn connect() -> Result<Self, String> {
        let url = std::env::var("SCENRA_OBS_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let (socket, _) = tungstenite::connect(&url).map_err(|e| {
            format!("can't reach OBS at {url} ({e}). Start OBS and enable Tools › WebSocket Server Settings.")
        })?;
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            // Switching or reloading a profile can keep OBS from answering for well over 10 s.
            stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(err)?;
        }
        let mut obs = Self { socket, next_id: 0 };
        let hello = obs.read_op(0)?;
        // No event subscriptions: Scenra only makes requests.
        let mut identify = json!({ "rpcVersion": 1, "eventSubscriptions": 0 });
        if let Some(auth) = hello["authentication"].as_object() {
            let password = secrets::obs_password().ok_or(format!(
                "OBS wants a password: store it with `cmdkey /generic:{} /user:obs /pass:<password>`",
                secrets::OBS_TARGET
            ))?;
            let salt = auth["salt"].as_str().unwrap_or_default();
            let challenge = auth["challenge"].as_str().unwrap_or_default();
            identify["authentication"] = json!(auth_string(&password, salt, challenge));
        }
        obs.send(json!({ "op": 1, "d": identify }))?;
        obs.read_op(2).map_err(|e| format!("OBS refused Scenra: {e}"))?;
        Ok(obs)
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        self.socket.send(Message::text(message.to_string())).map_err(err)
    }

    fn read(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read().map_err(err)? {
                Message::Text(text) => return serde_json::from_str(&text).map_err(err),
                Message::Close(frame) => {
                    let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                    return Err(format!("OBS closed the connection: {reason}"));
                }
                _ => {}
            }
        }
    }

    /// Reads messages until one with `op`, returning its `d`.
    fn read_op(&mut self, op: u64) -> Result<Value, String> {
        loop {
            let mut message = self.read()?;
            if message["op"] == op {
                return Ok(message["d"].take());
            }
        }
    }

    pub fn request(&mut self, kind: &str, data: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id.to_string();
        self.send(json!({ "op": 6, "d": { "requestType": kind, "requestId": id, "requestData": data } }))?;
        loop {
            let mut d = self.read_op(7)?;
            if d["requestId"] != id.as_str() {
                continue;
            }
            let status = &d["requestStatus"];
            if status["result"] != true {
                let comment = status["comment"].as_str().unwrap_or("no details");
                return Err(format!("OBS {kind} failed ({}): {comment}", status["code"]));
            }
            return Ok(d["responseData"].take());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked example from the obs-websocket v5 protocol docs.
    #[test]
    fn auth_string_matches_the_protocol_example() {
        let auth = auth_string("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=");
        assert_eq!(auth, "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=");
    }
}
