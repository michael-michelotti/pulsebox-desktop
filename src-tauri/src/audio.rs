use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use serde::Serialize;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;

// Must match wifi_audio.h on the ESP32
const UDP_PORT: u16 = 5000;
const SAMPLES_PER_PKT: usize = 512;
const MAGIC: u16 = 0x5042;

pub struct AudioState {
    inner: Mutex<Option<AudioStream>>,
    streaming: Arc<AtomicBool>,
}

struct AudioStream {
    _stream: Stream,
    device_name: String,
}

// SAFETY: cpal::Stream is Send but not marked as such on all platforms.
// On Windows (WASAPI), the stream callback runs on a separate thread and
// we only hold the Stream handle for start/stop. This is safe.
unsafe impl Send for AudioStream {}

impl AudioState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            streaming: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    pub name: String,
    pub index: usize,
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<AudioDevice>, String> {
    let host = cpal::host_from_id(cpal::HostId::Wasapi)
        .map_err(|e| format!("WASAPI not available: {}", e))?;

    let devices: Vec<AudioDevice> = host
        .output_devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?
        .enumerate()
        .filter_map(|(i, d)| {
            d.name().ok().map(|name| AudioDevice { name, index: i })
        })
        .collect();

    Ok(devices)
}

#[tauri::command]
pub fn start_audio_stream(
    ip: String,
    device_index: usize,
    state: State<'_, AudioState>,
) -> Result<String, String> {
    // Stop existing stream if any
    {
        let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
        *guard = None;
    }

    let host = cpal::host_from_id(cpal::HostId::Wasapi)
        .map_err(|e| format!("WASAPI not available: {}", e))?;

    let device: Device = host
        .output_devices()
        .map_err(|e| format!("Failed to enumerate devices: {}", e))?
        .nth(device_index)
        .ok_or("Device not found")?;

    let device_name = device.name().unwrap_or_default();

    let config = device
        .default_output_config()
        .map_err(|e| format!("No default output config: {}", e))?;

    let channels = config.channels() as usize;
    let sample_format = config.sample_format();

    let stream_config: StreamConfig = config.into();

    // Set up UDP socket
    let sock = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("UDP bind failed: {}", e))?;
    let target = format!("{}:{}", ip, UDP_PORT);
    sock.connect(&target)
        .map_err(|e| format!("UDP connect failed: {}", e))?;

    let seq = Arc::new(AtomicU16::new(0));
    let streaming = state.streaming.clone();
    streaming.store(true, Ordering::SeqCst);

    // Accumulation buffer for building complete packets
    let leftover: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(SAMPLES_PER_PKT)));

    let seq_clone = seq.clone();
    let streaming_clone = streaming.clone();

    let build_and_send = move |mono_samples: &[i16]| {
        if !streaming_clone.load(Ordering::SeqCst) {
            return;
        }

        let mut buf = leftover.lock().unwrap();
        buf.extend_from_slice(mono_samples);

        while buf.len() >= SAMPLES_PER_PKT {
            let chunk: Vec<i16> = buf.drain(..SAMPLES_PER_PKT).collect();

            let s = seq_clone.fetch_add(1, Ordering::SeqCst);
            let mut packet = Vec::with_capacity(4 + SAMPLES_PER_PKT * 2);
            packet.extend_from_slice(&MAGIC.to_le_bytes());
            packet.extend_from_slice(&s.to_le_bytes());
            for sample in &chunk {
                packet.extend_from_slice(&sample.to_le_bytes());
            }

            let _ = sock.send(&packet);
        }
    };

    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            let send_fn = build_and_send;
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<i16> = data
                        .chunks(channels)
                        .map(|frame| {
                            let sum: f32 = frame.iter().sum();
                            let avg = sum / channels as f32;
                            (avg * 32767.0).clamp(-32768.0, 32767.0) as i16
                        })
                        .collect();
                    send_fn(&mono);
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let send_fn = build_and_send;
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<i16> = data
                        .chunks(channels)
                        .map(|frame| {
                            let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                            (sum / channels as i32) as i16
                        })
                        .collect();
                    send_fn(&mono);
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            )
        }
        other => {
            return Err(format!("Unsupported sample format: {:?}", other));
        }
    }
    .map_err(|e| format!("Failed to build stream: {}", e))?;

    stream
        .play()
        .map_err(|e| format!("Failed to start stream: {}", e))?;

    let info = format!("Streaming from '{}' ({}ch) to {}", device_name, channels, target);

    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = Some(AudioStream {
        _stream: stream,
        device_name: device_name.clone(),
    });

    Ok(info)
}

#[tauri::command]
pub fn stop_audio_stream(state: State<'_, AudioState>) -> Result<(), String> {
    state.streaming.store(false, Ordering::SeqCst);
    let mut guard = state.inner.lock().map_err(|e| e.to_string())?;
    *guard = None;
    Ok(())
}

#[tauri::command]
pub fn is_audio_streaming(state: State<'_, AudioState>) -> Result<bool, String> {
    Ok(state.streaming.load(Ordering::SeqCst))
}
