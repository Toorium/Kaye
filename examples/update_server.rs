//! Serves a folder over plain HTTP, for trying updates without GitHub. A `devtools` build reads
//! `latest.json` from it when `KAYE_UPDATE_URL` points there.
//!
//!   cargo run --example update_server -- <folder> [port] [bytes per second]
//!
//! The speed limit makes a download slow enough to look at. Only 127.0.0.1 can reach it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CHUNK: usize = 64 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder = PathBuf::from(args.first().expect("usage: update_server <folder> [port] [bytes per second]"));
    let port: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(8765);
    let speed: Option<u64> = args.get(2).and_then(|s| s.parse().ok()).filter(|&s| s > 0);
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("that port is taken");
    println!("Serving {} at http://127.0.0.1:{port}/", folder.display());
    for stream in listener.incoming().flatten() {
        let folder = folder.clone();
        std::thread::spawn(move || answer(stream, &folder, speed));
    }
}

fn answer(mut stream: TcpStream, folder: &Path, speed: Option<u64>) {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
        request.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&request);
    let wanted = text.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").trim_start_matches('/').to_string();
    let file = (!wanted.split('/').any(|part| part == "..")).then(|| folder.join(&wanted)).filter(|p| p.is_file());
    let Some(bytes) = file.and_then(|p| std::fs::read(p).ok()) else {
        println!("404 /{wanted}");
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    println!("200 /{wanted} ({} bytes)", bytes.len());
    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for chunk in bytes.chunks(CHUNK) {
        if stream.write_all(chunk).is_err() {
            println!("   /{wanted} stopped partway");
            return;
        }
        if let Some(speed) = speed {
            std::thread::sleep(Duration::from_secs_f64(chunk.len() as f64 / speed as f64));
        }
    }
}
