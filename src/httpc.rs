//! The few HTTP calls aso makes, all to its own processes on this machine:
//! fsp-mint's `GET /health` and `GET /audit` on `server.bind`, and air_server's
//! health listener. A plain HTTP/1.0 GET over a loopback TCP connection.
//! Wallets reach the mint over AIR only; none of this is a wallet's path.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(5);

/// `GET http://addr/path`: the status code and the body. `addr` is `host:port`.
/// `Err` when nothing answers (the reason says what).
pub fn get(addr: &str, path: &str) -> Result<(u16, String), String> {
    let sock: SocketAddr = addr
        .to_socket_addrs()
        .map_err(|e| format!("`{addr}` is not an address: {e}"))?
        .next()
        .ok_or_else(|| format!("`{addr}` is not an address"))?;
    let mut stream = TcpStream::connect_timeout(&sock, TIMEOUT)
        .map_err(|e| format!("cannot reach {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| format!("cannot set the timeout on {addr}: {e}"))?;
    // One write: a listener that reads the request once (air_server's health
    // listener does) must get all of it, or its close resets the connection.
    let request = format!("GET {path} HTTP/1.0\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("cannot send to {addr}: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("cannot read from {addr}: {e}"))?;
    parse(&raw).ok_or_else(|| format!("{addr} did not answer with HTTP"))
}

/// The status and body of an HTTP response.
fn parse(raw: &[u8]) -> Option<(u16, String)> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((status, body.to_string()))
}

/// Whether a fsp-mint `GET /health` answer means it is sound: status 200 and
/// `"status":"ok"` in the JSON.
pub fn fsp_healthy(status: u16, body: &str) -> bool {
    status == 200
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("status").and_then(|s| s.as_str()).map(|s| s == "ok"))
            .unwrap_or(false)
}

/// Whether an air_server health answer is the bare `ok` it sends.
pub fn air_healthy(status: u16, body: &str) -> bool {
    status == 200 && body.trim() == "ok"
}
