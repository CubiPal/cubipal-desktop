use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareUpdateInfo {
    pub available: bool,
    pub latest_version: String,
    pub current_version: String,
    pub download_url: Option<String>,
    pub release_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub async fn check_firmware_update() -> Result<FirmwareUpdateInfo, String> {
    let dev = crate::device::get_paired_device()
        .await
        .map_err(|e| format!("Failed to read paired device: {}", e))?
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    let current_version = dev.fw.unwrap_or_else(|| "1.0.0".to_string());

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("CubiPal-Desktop")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get("https://api.github.com/repos/houndman1017/cubipal-desktop/releases/latest")
        .send()
        .await;

    let release: GithubRelease = match res {
        Ok(resp) if resp.status().is_success() => {
            resp.json().await.map_err(|e| e.to_string())?
        }
        _ => {
            return Ok(FirmwareUpdateInfo {
                available: false,
                latest_version: current_version.clone(),
                current_version,
                download_url: None,
                release_notes: None,
            });
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    let bin_asset = release.assets.into_iter().find(|a| {
        a.name.ends_with(".bin")
            || a.name.contains("firmware")
            || a.name.contains("cubipal")
    });

    let available = is_version_newer(&latest_version, &current_version);

    Ok(FirmwareUpdateInfo {
        available: available && bin_asset.is_some(),
        latest_version,
        current_version,
        download_url: bin_asset.map(|a| a.browser_download_url),
        release_notes: release.body,
    })
}

fn is_version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let l_parts = parse(latest);
    let c_parts = parse(current);
    for (l, c) in l_parts.iter().zip(c_parts.iter()) {
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    l_parts.len() > c_parts.len()
}

#[tauri::command]
pub async fn install_firmware_update(
    window: tauri::Window,
    download_url: String,
) -> Result<(), String> {
    emit_progress(&window, "downloading", 10, "Downloading firmware update from GitHub...");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("CubiPal-Desktop")
        .build()
        .map_err(|e| e.to_string())?;

    let bin_bytes = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download firmware: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read firmware data: {}", e))?
        .to_vec();

    emit_progress(&window, "uploading", 30, "Firmware downloaded. Uploading to CubiPal...");

    flash_firmware_bytes(&window, bin_bytes).await
}

#[tauri::command]
pub async fn install_local_firmware(
    window: tauri::Window,
    file_path: String,
) -> Result<(), String> {
    emit_progress(&window, "downloading", 10, "Reading local firmware file...");

    let bin_bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("Failed to read file '{}': {}", file_path, e))?;

    emit_progress(&window, "uploading", 30, "Uploading to CubiPal over Wi-Fi...");

    flash_firmware_bytes(&window, bin_bytes).await
}

async fn flash_firmware_bytes(
    window: &tauri::Window,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let (ip, token) = crate::device::get_saved_device_credentials()
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("firmware.bin")
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new().part("fw", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    emit_progress(window, "uploading", 60, "Writing firmware to CubiPal flash slot...");

    let res = client
        .post(format!("http://{}/api/update", ip))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Pet", "1")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {}", e))?;

    if res.status().is_success() {
        emit_progress(window, "rebooting", 95, "Flashing complete. CubiPal is rebooting...");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        emit_progress(window, "completed", 100, "Update successful! CubiPal is restarting.");
        Ok(())
    } else {
        let msg = format!("Device rejected update (HTTP {})", res.status());
        emit_progress(window, "error", 0, &msg);
        Err(msg)
    }
}

fn emit_progress(window: &tauri::Window, stage: &str, percent: u8, message: &str) {
    let _ = window.emit(
        "ota-progress",
        OtaProgress {
            stage: stage.to_string(),
            percent,
            message: message.to_string(),
        },
    );
}
