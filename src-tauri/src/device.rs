use keyring::Entry;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const KEYRING_SERVICE: &str = "cubipal";
const KEYRING_TOKEN_USER: &str = "device_token";
const KEYRING_IP_USER: &str = "device_ip";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPal {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub model: String,
    pub fw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceState {
    pub robot: String,
    pub owner: String,
    pub mood: Option<String>,
    pub volume: Option<u8>,
    pub muted: Option<bool>,
    pub ssid: Option<String>,
    pub rssi: Option<i32>,
    pub fw: Option<String>,
    pub ip: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub robot: Option<String>,
    pub owner: Option<String>,
    pub fw: Option<String>,
    pub msg: Option<String>,
}

#[tauri::command]
pub async fn scan_for_pals() -> Result<Vec<DiscoveredPal>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = mdns.browse("_cubipal._tcp.local.").map_err(|e| e.to_string())?;

    let mut pals = HashMap::new();
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(3);

    while start.elapsed() < timeout {
        if let Ok(event) = receiver.recv_timeout(Duration::from_millis(250)) {
            if let ServiceEvent::ServiceResolved(info) = event {
                let addrs = info.get_addresses();
                if let Some(ip) = addrs.iter().next() {
                    let id = info
                        .get_property_val_str("id")
                        .unwrap_or("unknown")
                        .to_string();
                    let name = info
                        .get_property_val_str("name")
                        .unwrap_or("Cubi")
                        .to_string();
                    let model = info
                        .get_property_val_str("model")
                        .unwrap_or("v6")
                        .to_string();
                    let fw = info
                        .get_property_val_str("fw")
                        .unwrap_or("1.0.0")
                        .to_string();

                    pals.insert(
                        id.clone(),
                        DiscoveredPal {
                            id,
                            name,
                            ip: ip.to_string(),
                            port: info.get_port(),
                            model,
                            fw,
                        },
                    );
                }
            }
        }
    }

    let _ = mdns.stop_browse("_cubipal._tcp.local.");
    Ok(pals.into_values().collect())
}

#[tauri::command]
pub async fn pair_pal(ip: String, code: String) -> Result<PairResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let clean_ip = ip.trim().trim_start_matches("http://").trim_end_matches('/');
    let url = format!("http://{}/api/pair", clean_ip);

    let clean_code = code.replace([' ', '-'], "");

    let payload = serde_json::json!({
        "code": clean_code,
        "clientName": "CubiPal Desktop"
    });

    let res = client
        .post(&url)
        .header("X-Pet", "1")
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Could not reach CubiPal at {}: {}", clean_ip, e))?;

    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let parsed: PairResponse = serde_json::from_str(&body)
        .map_err(|_| format!("Invalid response from device: {}", body))?;

    if status.is_success() && parsed.ok {
        if let Some(ref token) = parsed.token {
            store_device_credentials(clean_ip, token)?;
        }
        Ok(parsed)
    } else {
        let msg = parsed
            .msg
            .unwrap_or_else(|| format!("Pairing rejected (HTTP {})", status));
        Err(msg)
    }
}

#[tauri::command]
pub async fn get_paired_device() -> Result<Option<DeviceState>, String> {
    let (ip, token) = match get_saved_device_credentials() {
        Some(creds) => creds,
        None => return Ok(None),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("http://{}/api/state", ip);

    let res = match client
        .get(&url)
        .header("X-Pet", "1")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return Ok(Some(DeviceState {
                robot: "Cubi".to_string(),
                owner: "".to_string(),
                mood: None,
                volume: None,
                muted: None,
                ssid: None,
                rssi: None,
                fw: None,
                ip,
                connected: false,
            }));
        }
    };

    if !res.status().is_success() {
        return Ok(Some(DeviceState {
            robot: "Cubi".to_string(),
            owner: "".to_string(),
            mood: None,
            volume: None,
            muted: None,
            ssid: None,
            rssi: None,
            fw: None,
            ip,
            connected: false,
        }));
    }

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    Ok(Some(DeviceState {
        robot: val["robot"].as_str().unwrap_or("Cubi").to_string(),
        owner: val["owner"].as_str().unwrap_or("").to_string(),
        mood: val["mood"].as_str().map(String::from),
        volume: val["volume"].as_u64().map(|v| v as u8),
        muted: val["muted"].as_bool(),
        ssid: val["ssid"].as_str().map(String::from),
        rssi: val["rssi"].as_i64().map(|r| r as i32),
        fw: val["fw"].as_str().map(String::from),
        ip,
        connected: true,
    }))
}

#[tauri::command]
pub async fn unpair_pal() -> Result<(), String> {
    clear_device_credentials()
}

pub fn store_device_credentials(ip: &str, token: &str) -> Result<(), String> {
    let entry_tok = Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_USER).map_err(|e| e.to_string())?;
    entry_tok.set_password(token).map_err(|e| e.to_string())?;

    let entry_ip = Entry::new(KEYRING_SERVICE, KEYRING_IP_USER).map_err(|e| e.to_string())?;
    entry_ip.set_password(ip).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_saved_device_credentials() -> Option<(String, String)> {
    let entry_tok = Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_USER).ok()?;
    let token = entry_tok.get_password().ok()?;

    let entry_ip = Entry::new(KEYRING_SERVICE, KEYRING_IP_USER).ok()?;
    let ip = entry_ip.get_password().ok()?;

    Some((ip, token))
}

pub fn clear_device_credentials() -> Result<(), String> {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_USER) {
        let _ = entry.delete_credential();
    }
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_IP_USER) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

