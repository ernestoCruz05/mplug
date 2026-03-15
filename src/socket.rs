use crate::event::{WaylandEvent, WaylandRequest};
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::sync::mpsc::Sender;

pub const SOCKET_PATH: &str = "/tmp/mplug.sock";

pub fn run_socket(req_tx: Sender<WaylandRequest>, event_tx: Sender<WaylandEvent>) {
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = match UnixListener::bind(SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mplug: failed to bind socket {}: {}", SOCKET_PATH, e);
            return;
        }
    };

    println!("Socket IPC listening on {}", SOCKET_PATH);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let req_tx = req_tx.clone();
                let event_tx = event_tx.clone();
                std::thread::spawn(move || {
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        match line {
                            Ok(cmd) => parse_and_send(cmd.trim(), &req_tx, &event_tx),
                            Err(_) => break,
                        }
                    }
                });
            }
            Err(e) => eprintln!("mplug: socket accept error: {}", e),
        }
    }
}

fn parse_and_send(command: &str, req_tx: &Sender<WaylandRequest>, event_tx: &Sender<WaylandEvent>) {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }

    match parts[0] {
        "trigger" => {
            if parts.len() >= 2 {
                let name = parts[1..].join(" ");
                let _ = event_tx.send(WaylandEvent::UserCommand(name));
            } else {
                eprintln!("mplug socket: usage: trigger <name>");
            }
        }

        "set_tags" => {
            if let Some(Ok(tagmask)) = parts.get(1).map(|s| s.parse::<u32>()) {
                let _ = req_tx.send(WaylandRequest::SetTags(tagmask));
            } else {
                eprintln!("mplug socket: usage: set_tags <tagmask>");
            }
        }
        "set_layout" => {
            if let Some(Ok(index)) = parts.get(1).map(|s| s.parse::<u32>()) {
                let _ = req_tx.send(WaylandRequest::SetLayout(index));
            } else {
                eprintln!("mplug socket: usage: set_layout <index>");
            }
        }
        "focus_window" => {
            if let Some(Ok(id)) = parts.get(1).map(|s| s.parse::<u32>()) {
                let _ = req_tx.send(WaylandRequest::ActivateToplevel { id });
            } else {
                eprintln!("mplug socket: usage: focus_window <id>");
            }
        }
        "close_window" => {
            if let Some(Ok(id)) = parts.get(1).map(|s| s.parse::<u32>()) {
                let _ = req_tx.send(WaylandRequest::CloseToplevel { id });
            } else {
                eprintln!("mplug socket: usage: close_window <id>");
            }
        }
        "set_window_tag" => {
            match (
                parts.get(1).map(|s| s.parse::<u32>()),
                parts.get(2).map(|s| s.parse::<u32>()),
            ) {
                (Some(Ok(id)), Some(Ok(tagmask))) => {
                    let _ = req_tx.send(WaylandRequest::SetToplevelTags { id, tagmask });
                }
                _ => eprintln!("mplug socket: usage: set_window_tag <id> <tagmask>"),
            }
        }
        "set_client_tags" => {
            match (
                parts.get(1).map(|s| s.parse::<u32>()),
                parts.get(2).map(|s| s.parse::<u32>()),
            ) {
                (Some(Ok(and_tags)), Some(Ok(xor_tags))) => {
                    let _ = req_tx.send(WaylandRequest::SetClientTags { and_tags, xor_tags });
                }
                _ => eprintln!("mplug socket: usage: set_client_tags <and_tags> <xor_tags>"),
            }
        }
        "set_window_minimized" => {
            if parts.len() >= 3 {
                if let Ok(id) = parts[1].parse::<u32>() {
                    let minimized = matches!(parts[2], "true" | "1");
                    let _ = req_tx.send(WaylandRequest::SetToplevelMinimized { id, minimized });
                } else {
                    eprintln!("mplug socket: usage: set_window_minimized <id> <true|false>");
                }
            } else {
                eprintln!("mplug socket: usage: set_window_minimized <id> <true|false>");
            }
        }
        _ => eprintln!("mplug socket: unknown command: {}", command),
    }
}
