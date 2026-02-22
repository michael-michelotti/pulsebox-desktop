use serde::Serialize;
use std::sync::Mutex;
use tauri::State;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct ConnectionState {
    inner: Mutex<Option<ConnectionInfo>>,
}

struct ConnectionInfo {
    ip: String,
    port: u16,
    stream: TcpStream,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub ip: Option<String>,
    pub port: Option<u16>,
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn connect(
    ip: String,
    port: u16,
    state: State<'_, ConnectionState>,
) -> Result<(), String> {
    let addr = format!("{}:{}", ip, port);
    let stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = Some(ConnectionInfo { ip, port, stream });
    Ok(())
}

#[tauri::command]
pub async fn disconnect(state: State<'_, ConnectionState>) -> Result<(), String> {
    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}

#[tauri::command]
pub async fn send_command(
    cmd: String,
    state: State<'_, ConnectionState>,
) -> Result<String, String> {
    // Take the connection out of the mutex so we can do async I/O without holding the lock
    let mut conn = {
        let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
        guard.take().ok_or("Not connected")?
    };

    let result = async {
        // Send command with newline
        let cmd_line = format!("{}\n", cmd);
        conn.stream
            .write_all(cmd_line.as_bytes())
            .await
            .map_err(|e| format!("Send failed: {}", e))?;

        // Read response (all available lines within a short timeout)
        let mut reader = BufReader::new(&mut conn.stream);
        let mut response = String::new();

        match tokio::time::timeout(
            std::time::Duration::from_millis(500),
            reader.read_line(&mut response),
        )
        .await
        {
            Ok(Ok(0)) => return Err("Connection closed".to_string()),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(format!("Read failed: {}", e)),
            Err(_) => {} // timeout — no response is fine (e.g. for commands with no output)
        }

        Ok(response)
    }
    .await;

    // Put the connection back (unless it errored with a connection issue)
    match &result {
        Ok(_) => {
            let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
            *guard = Some(conn);
        }
        Err(msg) if msg.contains("Connection closed") => {
            // Don't put it back — connection is dead
        }
        Err(_) => {
            // Put it back anyway for non-fatal errors
            let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
            *guard = Some(conn);
        }
    }

    result
}

#[tauri::command]
pub async fn is_connected(state: State<'_, ConnectionState>) -> Result<ConnectionStatus, String> {
    let guard = state.inner.lock().map_err(|e| e.to_string())?;
    match &*guard {
        Some(conn) => Ok(ConnectionStatus {
            connected: true,
            ip: Some(conn.ip.clone()),
            port: Some(conn.port),
        }),
        None => Ok(ConnectionStatus {
            connected: false,
            ip: None,
            port: None,
        }),
    }
}
