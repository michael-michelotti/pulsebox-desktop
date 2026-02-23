use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

/* --- Protocol constants (must match ESP32 protocol.h) --- */

const MSG_HELLO: u8 = 0x01;
const MSG_CMD: u8 = 0x02;
const MSG_PIXEL_FRAME: u8 = 0x03;
const MSG_STATUS: u8 = 0x81;
const MSG_CMD_RESP: u8 = 0x82;

const PROTOCOL_VERSION: u8 = 1;
const STATUS_FIXED_SIZE: usize = 18;

/* --- Types --- */

pub struct ConnectionState {
    inner: Mutex<Option<ConnectionHandle>>,
}

/// Active connection: write half for sending, channel for CMD_RESP replies
struct ConnectionHandle {
    ip: String,
    port: u16,
    writer: OwnedWriteHalf,
    /// Send a oneshot sender here; the reader task will forward the next CMD_RESP through it
    resp_tx: mpsc::Sender<oneshot::Sender<(bool, String)>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStatus {
    pub connected: bool,
    pub ip: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub protocol_ver: u8,
    pub wifi_mode: String,
    pub brightness: u8,
    pub color_r: u8,
    pub color_g: u8,
    pub color_b: u8,
    pub speed: f32,
    pub direction: f32,
    pub grid_width: u8,
    pub grid_height: u8,
    pub num_pixels: u16,
    pub effect: String,
    pub palette: String,
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }
}

/* --- Frame helpers --- */

async fn send_frame(writer: &mut OwnedWriteHalf, msg_type: u8, payload: &[u8]) -> Result<(), String> {
    let len = payload.len() as u16;
    let header = [msg_type, (len & 0xFF) as u8, (len >> 8) as u8];
    writer
        .write_all(&header)
        .await
        .map_err(|e| format!("Send header failed: {}", e))?;
    if !payload.is_empty() {
        writer
            .write_all(payload)
            .await
            .map_err(|e| format!("Send payload failed: {}", e))?;
    }
    Ok(())
}

async fn recv_frame_from(reader: &mut OwnedReadHalf) -> Result<(u8, Vec<u8>), String> {
    let mut header = [0u8; 3];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("Read header failed: {}", e))?;
    let msg_type = header[0];
    let length = u16::from_le_bytes([header[1], header[2]]) as usize;
    let mut payload = vec![0u8; length];
    if length > 0 {
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|e| format!("Read payload failed: {}", e))?;
    }
    Ok((msg_type, payload))
}

/// Temporary helper to recv a frame on an unsplit TcpStream (used during HELLO handshake)
async fn recv_frame(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), String> {
    let mut header = [0u8; 3];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("Read header failed: {}", e))?;
    let msg_type = header[0];
    let length = u16::from_le_bytes([header[1], header[2]]) as usize;
    let mut payload = vec![0u8; length];
    if length > 0 {
        stream
            .read_exact(&mut payload)
            .await
            .map_err(|e| format!("Read payload failed: {}", e))?;
    }
    Ok((msg_type, payload))
}

fn parse_status(payload: &[u8]) -> Result<DeviceStatus, String> {
    if payload.len() < STATUS_FIXED_SIZE {
        return Err("STATUS payload too short".into());
    }

    let speed = f32::from_le_bytes(
        payload[6..10]
            .try_into()
            .map_err(|_| "Bad speed bytes")?,
    );
    let direction = f32::from_le_bytes(
        payload[10..14]
            .try_into()
            .map_err(|_| "Bad direction bytes")?,
    );
    let num_pixels = u16::from_le_bytes(
        payload[16..18]
            .try_into()
            .map_err(|_| "Bad num_pixels bytes")?,
    );

    let mut offset = STATUS_FIXED_SIZE;

    // Effect name (length-prefixed)
    if offset >= payload.len() {
        return Err("Missing effect name".into());
    }
    let ename_len = payload[offset] as usize;
    offset += 1;
    if offset + ename_len > payload.len() {
        return Err("Effect name truncated".into());
    }
    let effect = String::from_utf8_lossy(&payload[offset..offset + ename_len]).to_string();
    offset += ename_len;

    // Palette name (length-prefixed)
    if offset >= payload.len() {
        return Err("Missing palette name".into());
    }
    let pname_len = payload[offset] as usize;
    offset += 1;
    if offset + pname_len > payload.len() {
        return Err("Palette name truncated".into());
    }
    let palette = String::from_utf8_lossy(&payload[offset..offset + pname_len]).to_string();

    Ok(DeviceStatus {
        protocol_ver: payload[0],
        wifi_mode: if payload[1] == 0 {
            "STA".into()
        } else {
            "AP".into()
        },
        brightness: payload[2],
        color_r: payload[3],
        color_g: payload[4],
        color_b: payload[5],
        speed,
        direction,
        grid_width: payload[14],
        grid_height: payload[15],
        num_pixels,
        effect,
        palette,
    })
}

/* --- Background reader task --- */

/// Continuously reads frames from the server.
/// - MSG_STATUS → emits "device-status" Tauri event
/// - MSG_CMD_RESP → forwards to whoever is waiting via the oneshot channel
/// - On disconnect → emits "device-disconnected" event and exits
async fn reader_task(
    mut reader: OwnedReadHalf,
    mut resp_rx: mpsc::Receiver<oneshot::Sender<(bool, String)>>,
    app_handle: tauri::AppHandle,
) {
    loop {
        match recv_frame_from(&mut reader).await {
            Ok((msg_type, payload)) => match msg_type {
                MSG_STATUS => {
                    if let Ok(status) = parse_status(&payload) {
                        let _ = app_handle.emit("device-status", &status);
                    }
                }
                MSG_CMD_RESP => {
                    let success = payload.first().copied().unwrap_or(0) != 0;
                    let text = if payload.len() > 1 {
                        String::from_utf8_lossy(&payload[1..]).to_string()
                    } else {
                        String::new()
                    };
                    // Try to deliver to a waiting send_command caller
                    if let Ok(reply_tx) = resp_rx.try_recv() {
                        let _ = reply_tx.send((success, text));
                    }
                }
                _ => {
                    // Ignore unknown message types
                }
            },
            Err(_) => {
                // Connection lost
                let _ = app_handle.emit("device-disconnected", ());
                break;
            }
        }
    }
}

/* --- Tauri commands --- */

#[tauri::command]
pub async fn connect(
    ip: String,
    port: u16,
    app_handle: tauri::AppHandle,
    state: State<'_, ConnectionState>,
) -> Result<DeviceStatus, String> {
    let addr = format!("{}:{}", ip, port);
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    // Send HELLO on the unsplit stream
    let len: u16 = 1;
    let header = [MSG_HELLO, (len & 0xFF) as u8, (len >> 8) as u8];
    stream
        .write_all(&header)
        .await
        .map_err(|e| format!("Send HELLO failed: {}", e))?;
    stream
        .write_all(&[PROTOCOL_VERSION])
        .await
        .map_err(|e| format!("Send HELLO payload failed: {}", e))?;

    // Read STATUS response on the unsplit stream
    let (msg_type, payload) = tokio::time::timeout(
        std::time::Duration::from_millis(2000),
        recv_frame(&mut stream),
    )
    .await
    .map_err(|_| "Timeout waiting for STATUS response".to_string())??;

    if msg_type != MSG_STATUS {
        return Err(format!("Expected STATUS (0x81), got 0x{:02x}", msg_type));
    }
    let status = parse_status(&payload)?;

    // Split the stream and spawn the background reader
    let (reader, writer) = stream.into_split();
    let (resp_tx, resp_rx) = mpsc::channel::<oneshot::Sender<(bool, String)>>(16);

    tokio::spawn(reader_task(reader, resp_rx, app_handle));

    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = Some(ConnectionHandle {
        ip,
        port,
        writer,
        resp_tx,
    });

    Ok(status)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, ConnectionState>) -> Result<(), String> {
    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    // Dropping the ConnectionHandle closes the write half,
    // which causes the reader task to get an error and exit
    *guard = None;
    Ok(())
}

#[tauri::command]
pub async fn send_command(
    cmd: String,
    state: State<'_, ConnectionState>,
) -> Result<(bool, String), String> {
    // Take the handle out briefly to do async I/O
    let mut handle = {
        let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
        guard.take().ok_or("Not connected")?
    };

    // Set up a oneshot channel for the CMD_RESP
    let (reply_tx, reply_rx) = oneshot::channel::<(bool, String)>();

    let result: Result<(bool, String), String> = async {
        // Register our reply channel with the reader task
        handle
            .resp_tx
            .send(reply_tx)
            .await
            .map_err(|_| "Reader task gone (disconnected)".to_string())?;

        // Send CMD frame
        send_frame(&mut handle.writer, MSG_CMD, cmd.as_bytes()).await?;

        // Wait for the reader task to forward the CMD_RESP
        let resp = tokio::time::timeout(std::time::Duration::from_millis(2000), reply_rx)
            .await
            .map_err(|_| "Response timeout".to_string())?
            .map_err(|_| "Reader task gone (disconnected)".to_string())?;

        Ok(resp)
    }
    .await;

    // Put the handle back (unless connection is dead)
    match &result {
        Err(msg) if msg.contains("disconnected") || msg.contains("Send") => {
            // Connection is dead, don't put it back
        }
        _ => {
            let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
            *guard = Some(handle);
        }
    }

    result
}

#[tauri::command]
pub async fn send_pixel_frame(
    pixels: Vec<u8>,
    state: State<'_, ConnectionState>,
) -> Result<(), String> {
    let mut handle = {
        let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
        guard.take().ok_or("Not connected")?
    };

    let result = send_frame(&mut handle.writer, MSG_PIXEL_FRAME, &pixels).await;

    // Always put handle back
    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = Some(handle);

    result
}

#[tauri::command]
pub async fn is_connected(state: State<'_, ConnectionState>) -> Result<ConnectionStatus, String> {
    let guard = state.inner.lock().map_err(|e| e.to_string())?;
    match &*guard {
        Some(handle) => Ok(ConnectionStatus {
            connected: true,
            ip: Some(handle.ip.clone()),
            port: Some(handle.port),
        }),
        None => Ok(ConnectionStatus {
            connected: false,
            ip: None,
            port: None,
        }),
    }
}
