//! The chat page on this machine: a small HTTP server on the standard library.
//!
//!   GET  /        the page (ui/index.html, built into the binary)
//!   POST /start   a new conversation: {"text": greeting}
//!   POST /ask     {"q": "..."} -> {"text": "...", "why": ["dog -is a-> animal", ...], "ms": 12}
//!
//! One conversation at a time, answered in turn (it is a local page).

use crate::chat::Chat;
use crate::config::Config;
use crate::network::Network;
use anyhow::Result;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Instant;

const PAGE: &str = include_str!("../ui/index.html");

struct Request {
    method: String,
    path: String,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length.min(1 << 16)];
    reader.read_exact(&mut body)?;
    Ok(Request { method, path, body: String::from_utf8_lossy(&body).into_owned() })
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

pub fn serve(net: &Network, cfg: &Config, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("Primo is listening: http://127.0.0.1:{port}/  (ctrl+c to stop)");
    let mut chat = Chat::new(net, cfg);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Ok(req) = read_request(&mut stream) else { continue };
        let result = match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/") | ("GET", "/index.html") => respond(&mut stream, "200 OK", "text/html", PAGE),
            ("POST", "/start") => {
                chat = Chat::new(net, cfg);
                respond(&mut stream, "200 OK", "application/json", &json!({ "text": cfg.chat.greeting }).to_string())
            }
            ("POST", "/ask") => {
                let q = serde_json::from_str::<serde_json::Value>(&req.body)
                    .ok()
                    .and_then(|v| v.get("q").and_then(|q| q.as_str()).map(|s| s.to_string()))
                    .unwrap_or_default();
                let t = Instant::now();
                let r = chat.reply(&q);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                respond(&mut stream, "200 OK", "application/json", &json!({ "text": r.text, "why": r.why, "ms": ms }).to_string())
            }
            _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
        };
        if let Err(e) = result {
            eprintln!("  answering failed: {e}");
        }
    }
    Ok(())
}
