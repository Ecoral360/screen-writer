//! Single-instance control socket: `screen-writer toggle` etc. talk to the
//! running daemon through a Unix socket in `$XDG_RUNTIME_DIR`.

use crate::app::{Cmd, Msg};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let display = std::env::var("DISPLAY").unwrap_or_default().replace(['/', ':'], "_");
    dir.join(format!("screen-writer{display}.sock"))
}

/// Send a command line to a running instance. Returns `false` if none is running.
pub fn send(cmd: &str) -> bool {
    let Ok(mut s) = UnixStream::connect(socket_path()) else { return false };
    if writeln!(s, "{cmd}").is_err() {
        return false;
    }
    let mut reply = String::new();
    let _ = BufReader::new(s).read_line(&mut reply);
    if reply.starts_with("err") {
        eprintln!("screen-writer: {}", reply.trim());
    }
    true
}

/// Bind the control socket. Fails if another instance is already listening.
pub fn bind() -> Result<UnixListener, String> {
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        return Err("screen-writer is already running".into());
    }
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).map_err(|e| format!("cannot bind {}: {e}", path.display()))
}

pub fn serve(listener: UnixListener, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut out = &stream;
            match Cmd::parse(line.trim()) {
                Some(c) => {
                    let quit = c == Cmd::Quit;
                    let _ = writeln!(out, "ok");
                    if tx.send(Msg::Cmd(c)).is_err() || quit {
                        if quit {
                            continue;
                        }
                        break;
                    }
                }
                None => {
                    let _ = writeln!(out, "err unknown command: {}", line.trim());
                }
            }
        }
    });
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}
