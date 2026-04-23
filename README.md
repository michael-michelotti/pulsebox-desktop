# PulseBox Desktop

**Companion desktop app for the [Pulse Box](https://github.com/michael-michelotti/pulse-box-esp) LED controller**

[PulseBox Desktop in action](https://github.com/user-attachments/assets/6c7008e8-37e6-4140-984e-331421be73a4)

A cross-platform desktop app for the ESP32-S3-based Pulse Box controller. Discovers PulseBox controller on the local network via mDNS, sends/receives commands/status over a binary TLV protocol over TCP, streams desktop audio for audio-reactive effects, uploads images for pixel-level display, and renders a live preview of the LED grid — including dynamic multi-panel topologies.

`Tauri v2` | `Rust` | `SvelteKit` | `TypeScript` | `WASAPI Loopback` | `mDNS Discovery`

---

## Features

- **Device discovery** — scans for `_pulsebox-cmd._tcp` services via mDNS, with manual connect fallback for AP-mode setup
- **Control UI** — effect selection, palette picker, brightness/speed/direction/sensitivity sliders, color configuration
- **WiFi provisioning** — first-time setup flow for pushing home WiFi credentials to a controller in AP mode
- **System audio streaming** — captures desktop audio via WASAPI loopback and streams it to the controller as 48 kHz PCM over UDP
- **Image upload** — loads, resizes, and streams still images to the LED grid as raw pixel frames
- **Live preview** — renders a preview of the controller's framebuffer at ~1 Hz, scaling to any multi-panel topology with accurate panel boundaries

![PulseBox Desktop UI](https://github.com/user-attachments/assets/d16c41aa-1ab7-4f79-9314-c78ea1e67a18)
_The desktop app connected to a controller, showing device discovery, effect controls, and live LED preview._

---

## Tech Stack

Built with **Tauri v2** (Rust backend, SvelteKit + TypeScript frontend). Tauri was chosen over Electron for two reasons: direct access to native Windows audio APIs (WASAPI loopback capture) from the Rust backend, and a much smaller binary footprint. Svelte 5 on the frontend keeps the reactive UI code compact — state-heavy views like the live preview canvas and the multi-slider control panel are a good fit for Svelte's reactivity, and the runtime overhead is low enough that preview frames can be repainted without lag.

All network I/O, audio capture, and protocol handling live in Rust (the TCP TLV client, mDNS browser, WASAPI capture thread, UDP audio sender), and the Svelte frontend only renders UI state and the preview canvas. The two sides communicate through Tauri commands (frontend → backend) and Tauri events (backend → frontend, e.g. `status-update`, `preview-frame`, `device-discovered`). This keeps the frontend free of any socket or protocol logic.

---

## Project Structure

```
pulsebox-desktop/
├── src/                            # SvelteKit frontend
│   ├── app.html
│   └── routes/
│       ├── +layout.ts
│       └── +page.svelte            # Single-page control UI + live preview canvas
└── src-tauri/                      # Rust backend
    ├── Cargo.toml
    ├── tauri.conf.json
    └── src/
        ├── main.rs                 # Tauri entry point
        ├── lib.rs                  # App setup, command registration, event wiring
        ├── commands.rs             # Tauri commands: connect, send_cmd, upload_image, etc.
        ├── discovery.rs            # mDNS browser for _pulsebox-cmd._tcp
        └── audio.rs                # WASAPI loopback capture + UDP streaming
```

---

## Development

```bash
npm install
npm run tauri dev      # Hot-reload dev build
npm run tauri build    # Release bundle
```

Requires Rust (via [rustup](https://rustup.rs/)) and Node.js 18+. See the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for platform-specific toolchain setup.
