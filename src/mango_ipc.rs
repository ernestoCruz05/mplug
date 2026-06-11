use crate::event::WaylandEvent;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const WATCH_TOPICS: &[&str] = &[
    "keymode",
    "keyboardlayout",
    "all-monitors",
    "all-clients",
    "all-tags",
];

const IPC_TIMEOUT: Duration = Duration::from_millis(500);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub fn send_command(cmd: &str) -> Result<String> {
    let socket_path = std::env::var("MANGO_INSTANCE_SIGNATURE")
        .map_err(|_| anyhow!("MANGO_INSTANCE_SIGNATURE is not set"))?;
    send_command_to(&socket_path, cmd, IPC_TIMEOUT)
}

fn send_command_to(path: &str, cmd: &str, timeout: Duration) -> Result<String> {
    let mut stream = UnixStream::connect(path)
        .map_err(|e| anyhow!("Failed to connect to Mango socket {}: {}", path, e))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let cmd_with_nl = format!("{}\n", cmd);
    stream.write_all(cmd_with_nl.as_bytes())?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).map_err(|e| {
        anyhow!(
            "No reply from Mango socket {} within {:?}: {}",
            path,
            timeout,
            e
        )
    })?;

    Ok(response.trim_end().to_string())
}

pub fn parse_watch_line(line: &str) -> Option<Value> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(line).ok()
}

pub fn topic_event(topic: &str, val: Value) -> Option<WaylandEvent> {
    match topic {
        "keymode" => val
            .get("keymode")
            .and_then(|v| v.as_str())
            .map(|m| WaylandEvent::IpcKeyMode(m.to_string())),
        "keyboardlayout" => val
            .get("layout")
            .and_then(|v| v.as_str())
            .map(|l| WaylandEvent::IpcKeyboardLayout(l.to_string())),
        "all-monitors" => Some(WaylandEvent::IpcMonitors(val)),
        "all-clients" => Some(WaylandEvent::IpcClients(val)),
        "all-tags" => Some(WaylandEvent::IpcTags(val)),
        _ => None,
    }
}

fn spawn_watch_loop(
    topic: String,
    event_tx: Sender<WaylandEvent>,
    to_event: impl Fn(Value) -> Option<WaylandEvent> + Send + 'static,
) {
    std::thread::spawn(move || {
        let socket_path = match std::env::var("MANGO_INSTANCE_SIGNATURE") {
            Ok(path) => path,
            Err(_) => return,
        };

        loop {
            match UnixStream::connect(&socket_path) {
                Ok(mut stream) => {
                    let cmd = format!("watch {}\n", topic);
                    if let Err(e) = stream.write_all(cmd.as_bytes()) {
                        crate::log_error!(
                            "mango-ipc",
                            "failed to write watch for {}: {}",
                            topic,
                            e
                        );
                    } else {
                        let reader = BufReader::new(stream);
                        for line in reader.lines() {
                            let Ok(text) = line else { break };
                            if let Some(value) = parse_watch_line(&text) {
                                if let Some(event) = to_event(value) {
                                    if event_tx.send(event).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    std::thread::sleep(RECONNECT_DELAY);
                }
                Err(_) => std::thread::sleep(RECONNECT_DELAY),
            }
        }
    });
}

pub fn start_callback_watch(topic: String, id: u64, event_tx: Sender<WaylandEvent>) {
    spawn_watch_loop(topic, event_tx, move |value| {
        Some(WaylandEvent::WatchUpdate { id, value })
    });
}

pub fn start_watch_thread(topic: &'static str, event_tx: Sender<WaylandEvent>) {
    spawn_watch_loop(topic.to_string(), event_tx, move |value| {
        topic_event(topic, value)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn parse_watch_line_parses_json_object() {
        let v = parse_watch_line(r#"{"title":"foo"}"#).unwrap();
        assert_eq!(v.get("title").and_then(|x| x.as_str()), Some("foo"));
    }

    #[test]
    fn parse_watch_line_skips_non_json_and_blank() {
        assert!(parse_watch_line("not json").is_none());
        assert!(parse_watch_line("").is_none());
        assert!(parse_watch_line("   ").is_none());
    }

    #[test]
    fn send_command_to_returns_server_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mango.sock");
        let listener = UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut stream = stream;
            stream.write_all(b"{\"success\":true}\n").unwrap();
            line
        });

        let response = send_command_to(
            path.to_str().unwrap(),
            "dispatch zoom",
            Duration::from_millis(500),
        )
        .unwrap();

        assert_eq!(response, "{\"success\":true}");
        assert_eq!(server.join().unwrap(), "dispatch zoom\n");
    }

    #[test]
    fn send_command_to_errors_instead_of_hanging_when_server_never_replies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mango.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        let start = std::time::Instant::now();
        let result = send_command_to(
            path.to_str().unwrap(),
            "get all-clients",
            Duration::from_millis(100),
        );

        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn topic_event_maps_keymode_and_layout() {
        let val: Value = serde_json::json!({"keymode": "normal"});
        assert!(matches!(
            topic_event("keymode", val),
            Some(WaylandEvent::IpcKeyMode(m)) if m == "normal"
        ));

        let val: Value = serde_json::json!({"layout": "us"});
        assert!(matches!(
            topic_event("keyboardlayout", val),
            Some(WaylandEvent::IpcKeyboardLayout(l)) if l == "us"
        ));
    }

    #[test]
    fn topic_event_skips_missing_fields_and_unknown_topics() {
        assert!(topic_event("keymode", serde_json::json!({"other": 1})).is_none());
        assert!(topic_event("keyboardlayout", serde_json::json!({})).is_none());
        assert!(topic_event("not-a-topic", serde_json::json!({})).is_none());
    }

    #[test]
    fn topic_event_wraps_state_topics() {
        let val = serde_json::json!({"monitors": []});
        assert!(matches!(
            topic_event("all-monitors", val),
            Some(WaylandEvent::IpcMonitors(_))
        ));
        assert!(matches!(
            topic_event("all-clients", serde_json::json!([])),
            Some(WaylandEvent::IpcClients(_))
        ));
        assert!(matches!(
            topic_event("all-tags", serde_json::json!([])),
            Some(WaylandEvent::IpcTags(_))
        ));
    }
}
