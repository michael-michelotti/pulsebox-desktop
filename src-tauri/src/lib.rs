mod audio;
mod commands;
mod discovery;

use audio::AudioState;
use commands::ConnectionState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(ConnectionState::new())
        .manage(AudioState::new())
        .invoke_handler(tauri::generate_handler![
            discovery::discover_devices,
            commands::connect,
            commands::disconnect,
            commands::send_command,
            commands::is_connected,
            commands::send_pixel_frame,
            commands::process_image,
            commands::process_gif,
            audio::list_audio_devices,
            audio::start_audio_stream,
            audio::stop_audio_stream,
            audio::is_audio_streaming,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
