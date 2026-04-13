use base64::Engine;
use image::{AnimationDecoder, GenericImageView, imageops::FilterType};
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
const MSG_PREVIEW_FRAME: u8 = 0x83;

const PROTOCOL_VERSION: u8 = 1;
const STATUS_FIXED_SIZE: usize = 25;

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
pub struct PanelPosition {
    pub grid_x: u8,
    pub grid_y: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub protocol_ver: u8,
    pub wifi_mode: String,
    pub brightness: u8,
    pub color_r: u8,
    pub color_g: u8,
    pub color_b: u8,
    pub color2_r: u8,
    pub color2_g: u8,
    pub color2_b: u8,
    pub color3_r: u8,
    pub color3_g: u8,
    pub color3_b: u8,
    pub speed: f32,
    pub direction: f32,
    pub grid_width: u8,
    pub grid_height: u8,
    pub num_pixels: u16,
    pub sensitivity: u8,
    pub effect: String,
    pub palette: String,
    pub panels: Vec<PanelPosition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessedImage {
    /// Raw RGB pixel data, row-major, grid_w * grid_h * 3 bytes
    pub pixels: Vec<u8>,
    /// Base64-encoded PNG of the processed image (upscaled for preview)
    pub preview_png_base64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GifFrame {
    pub pixels: Vec<u8>,
    pub delay_ms: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessedGif {
    pub frames: Vec<GifFrame>,
    pub preview_png_base64: String,
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
        payload[12..16]
            .try_into()
            .map_err(|_| "Bad speed bytes")?,
    );
    let direction = f32::from_le_bytes(
        payload[16..20]
            .try_into()
            .map_err(|_| "Bad direction bytes")?,
    );
    let num_pixels = u16::from_le_bytes(
        payload[22..24]
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
    offset += pname_len;

    // Panel topology (optional — backward compatible with older firmware)
    let mut panels = vec![PanelPosition { grid_x: 0, grid_y: 0 }]; // controller always at (0,0)
    if offset < payload.len() {
        let num_panels = payload[offset] as usize;
        offset += 1;
        panels.clear();
        for _ in 0..num_panels {
            if offset + 2 > payload.len() {
                break;
            }
            panels.push(PanelPosition {
                grid_x: payload[offset],
                grid_y: payload[offset + 1],
            });
            offset += 2;
        }
    }

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
        color2_r: payload[6],
        color2_g: payload[7],
        color2_b: payload[8],
        color3_r: payload[9],
        color3_g: payload[10],
        color3_b: payload[11],
        speed,
        direction,
        grid_width: payload[20],
        grid_height: payload[21],
        num_pixels,
        sensitivity: payload[24],
        effect,
        palette,
        panels,
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
                MSG_PREVIEW_FRAME => {
                    let _ = app_handle.emit("preview-frame", &payload);
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

#[tauri::command]
pub fn process_image(
    path: String,
    grid_width: u8,
    grid_height: u8,
) -> Result<ProcessedImage, String> {
    let img = image::open(&path).map_err(|e| format!("Failed to open image: {}", e))?;

    // Center-crop to square
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let x_offset = (w - side) / 2;
    let y_offset = (h - side) / 2;
    let cropped = img.crop_imm(x_offset, y_offset, side, side);

    // Bin-average downscale: divide image into grid-sized bins, average each bin's RGB
    let gw = grid_width as u32;
    let gh = grid_height as u32;
    let cropped_rgb = cropped.to_rgb8();
    let (cw, ch) = (side, side);
    let mut pixels: Vec<u8> = vec![0; (gw * gh * 3) as usize];

    for gy in 0..gh {
        for gx in 0..gw {
            let x0 = gx * cw / gw;
            let x1 = ((gx + 1) * cw / gw).min(cw);
            // Flip vertically: image row 0 is top, LED grid row 0 is bottom
            let src_y0 = (gh - 1 - gy) * ch / gh;
            let src_y1 = ((gh - gy) * ch / gh).min(ch);

            let mut r_sum: u64 = 0;
            let mut g_sum: u64 = 0;
            let mut b_sum: u64 = 0;
            let mut count: u64 = 0;

            for sy in src_y0..src_y1 {
                for sx in x0..x1 {
                    let p = cropped_rgb.get_pixel(sx, sy);
                    r_sum += p[0] as u64;
                    g_sum += p[1] as u64;
                    b_sum += p[2] as u64;
                    count += 1;
                }
            }

            if count > 0 {
                let idx = ((gy * gw + gx) * 3) as usize;
                pixels[idx + 0] = (r_sum / count) as u8;
                pixels[idx + 1] = (g_sum / count) as u8;
                pixels[idx + 2] = (b_sum / count) as u8;
            }
        }
    }

    // Generate preview from bin-averaged pixels (flip back to screen coordinates)
    let mut preview_img = image::RgbImage::new(gw, gh);
    for gy in 0..gh {
        for gx in 0..gw {
            let src_y = gh - 1 - gy; // un-flip for screen coordinates
            let idx = ((src_y * gw + gx) * 3) as usize;
            preview_img.put_pixel(gx, gy, image::Rgb([pixels[idx], pixels[idx + 1], pixels[idx + 2]]));
        }
    }
    let preview = image::DynamicImage::ImageRgb8(preview_img)
        .resize_exact(128, 128, FilterType::Nearest);
    let mut png_buf: Vec<u8> = Vec::new();
    preview
        .write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Failed to encode preview: {}", e))?;
    let preview_png_base64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);

    Ok(ProcessedImage {
        pixels,
        preview_png_base64,
    })
}

#[tauri::command]
pub fn process_gif(
    path: String,
    grid_width: u8,
    grid_height: u8,
) -> Result<ProcessedGif, String> {
    let file = std::fs::File::open(&path).map_err(|e| format!("Failed to open file: {}", e))?;
    let reader = std::io::BufReader::new(file);
    let decoder = image::codecs::gif::GifDecoder::new(reader)
        .map_err(|e| format!("Failed to decode GIF: {}", e))?;
    let raw_frames = decoder.into_frames().collect_frames()
        .map_err(|e| format!("Failed to read GIF frames: {}", e))?;

    let gw = grid_width as u32;
    let gh = grid_height as u32;
    let mut frames = Vec::new();
    let mut preview_png_base64 = String::new();

    for (i, frame) in raw_frames.into_iter().enumerate() {
        // Extract delay (numerator/denominator ratio in milliseconds)
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = if denom == 0 { 100 } else { numer / denom };
        // GIF spec: delay of 0 is often treated as 100ms
        let delay_ms = if delay_ms == 0 { 100 } else { delay_ms };

        let img = image::DynamicImage::from(frame.into_buffer());

        // Center-crop to square
        let (w, h) = img.dimensions();
        let side = w.min(h);
        let x_offset = (w - side) / 2;
        let y_offset = (h - side) / 2;
        let cropped = img.crop_imm(x_offset, y_offset, side, side);

        // Resize to grid dimensions
        let resized = cropped.resize_exact(gw, gh, FilterType::Lanczos3);

        // Generate preview from first frame (non-flipped, screen coordinates)
        if i == 0 {
            let preview = resized.resize_exact(128, 128, FilterType::Nearest);
            let mut png_buf: Vec<u8> = Vec::new();
            preview
                .write_to(
                    &mut std::io::Cursor::new(&mut png_buf),
                    image::ImageFormat::Png,
                )
                .map_err(|e| format!("Failed to encode preview: {}", e))?;
            preview_png_base64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
        }

        // Flip vertically for LED grid (row 0 = bottom)
        let flipped = resized.flipv();
        let rgb_image = flipped.to_rgb8();
        let pixels: Vec<u8> = rgb_image.as_raw().clone();

        frames.push(GifFrame { pixels, delay_ms });
    }

    if frames.is_empty() {
        return Err("GIF contains no frames".into());
    }

    Ok(ProcessedGif {
        frames,
        preview_png_base64,
    })
}
