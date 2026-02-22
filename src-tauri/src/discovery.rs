use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct PulseBoxDevice {
    pub name: String,
    pub ip: String,
    pub cmd_port: u16,
    pub audio_port: u16,
}

#[tauri::command]
pub async fn discover_devices() -> Result<Vec<PulseBoxDevice>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| format!("mDNS init failed: {}", e))?;

    // Browse for the TCP command service
    let service_type = "_pulsebox-cmd._tcp.local.";
    let receiver = mdns
        .browse(service_type)
        .map_err(|e| format!("mDNS browse failed: {}", e))?;

    let mut devices: HashMap<String, PulseBoxDevice> = HashMap::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);

    while std::time::Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let name = info.get_hostname().trim_end_matches('.').to_string();
                if let Some(ip) = info.get_addresses().iter().next() {
                    devices.insert(
                        name.clone(),
                        PulseBoxDevice {
                            name,
                            ip: ip.to_string(),
                            cmd_port: info.get_port(),
                            audio_port: 5000, // known constant
                        },
                    );
                }
            }
            Ok(_) => {} // ignore other events
            Err(_) => {} // timeout, keep waiting
        }
    }

    mdns.shutdown().ok();

    Ok(devices.into_values().collect())
}
