use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareUpdateInfo {
    pub available: bool,
    pub latest_version: String,
    pub current_version: String,
    pub download_url: Option<String>,
    pub sha256: Option<String>,
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

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FirmwareManifest {
    version: Option<String>,
    sha256: Option<String>,
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

    // Check official CubiPal firmware repo first, fallback to desktop repo releases
    let endpoints = [
        "https://api.github.com/repos/houndman1017/CubiPal/releases/latest",
        "https://api.github.com/repos/houndman1017/cubipal-desktop/releases/latest",
    ];

    let mut release_opt: Option<GithubRelease> = None;
    for endpoint in endpoints {
        if let Ok(resp) = client.get(endpoint).send().await
            && resp.status().is_success()
            && let Ok(rel) = resp.json::<GithubRelease>().await
        {
            release_opt = Some(rel);
            break;
        }
    }

    let release = match release_opt {
        Some(r) => r,
        None => {
            return Ok(FirmwareUpdateInfo {
                available: false,
                latest_version: current_version.clone(),
                current_version,
                download_url: None,
                sha256: None,
                release_notes: None,
            });
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    let mut bin_url = None;
    let mut manifest_url = None;
    let mut sha256_url = None;

    for asset in release.assets {
        let name_lower = asset.name.to_lowercase();
        if name_lower.ends_with(".bin") {
            bin_url = Some(asset.browser_download_url);
        } else if name_lower == "manifest.json" || name_lower.ends_with("-manifest.json") {
            manifest_url = Some(asset.browser_download_url);
        } else if name_lower.contains("sha256") {
            sha256_url = Some(asset.browser_download_url);
        }
    }

    // Try to fetch expected SHA-256 from manifest or checksum file if available
    let mut expected_sha256 = None;
    if let Some(m_url) = manifest_url
        && let Ok(resp) = client.get(m_url).send().await
        && let Ok(manifest) = resp.json::<FirmwareManifest>().await
    {
        expected_sha256 = manifest.sha256;
    }

    if expected_sha256.is_none()
        && let Some(s_url) = sha256_url
        && let Ok(resp) = client.get(s_url).send().await
        && let Ok(text) = resp.text().await
    {
        // Extract first 64-hex-char token
        for word in text.split_whitespace() {
            if word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit()) {
                expected_sha256 = Some(word.to_lowercase());
                break;
            }
        }
    }

    let available = is_version_newer(&latest_version, &current_version);

    Ok(FirmwareUpdateInfo {
        available: available && bin_url.is_some(),
        latest_version,
        current_version,
        download_url: bin_url,
        sha256: expected_sha256,
        release_notes: release.body,
    })
}

fn is_version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
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

// Ensure the device is not in the middle of a meeting or timer before updating (KAN-26 UX requirement)
async fn check_device_idle(
    client: &reqwest::Client,
    ip: &str,
    token: &str,
    force: bool,
) -> Result<(), String> {
    if force {
        return Ok(());
    }

    let res = client
        .get(format!("http://{}/api/state", ip))
        .header("X-Pet", "1")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    if let Ok(resp) = res
        && let Ok(state) = resp.json::<serde_json::Value>().await
    {
        // Check active timer
        let timer_sec = state["timerSec"].as_u64().unwrap_or(0);
        if timer_sec > 0 {
            return Err("Cannot install update: a Focus or Countdown timer is currently active on CubiPal. Stop the timer or wait for it to complete first.".to_string());
        }

        // Check upcoming or ongoing meeting
        if let Some(next_event) = state.get("nextEvent").filter(|e| !e.is_null()) {
            let start_epoch = next_event["start"].as_i64().unwrap_or(0);
            let now_epoch = chrono::Utc::now().timestamp();
            // Meeting is within 5 minutes or in progress (<1hr past start)
            if start_epoch > 0
                && (start_epoch - now_epoch) <= 300
                && (now_epoch - start_epoch) < 3600
            {
                let title = next_event["title"].as_str().unwrap_or("Meeting");
                return Err(format!(
                    "Cannot install update: calendar meeting '{}' is starting or in progress. Updates are paused to prevent desk interruptions.",
                    title
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn install_firmware_update(
    window: tauri::Window,
    download_url: String,
    expected_sha256: Option<String>,
    force: Option<bool>,
) -> Result<(), String> {
    emit_progress(
        &window,
        "downloading",
        10,
        "Downloading firmware update from GitHub...",
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("CubiPal-Desktop")
        .build()
        .map_err(|e| e.to_string())?;

    let (ip, token) = crate::device::get_saved_device_credentials()
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    // Check device state
    if let Err(err_msg) = check_device_idle(&client, &ip, &token, force.unwrap_or(false)).await {
        emit_progress(&window, "error", 0, &err_msg);
        return Err(err_msg);
    }

    let bin_bytes = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download firmware: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read firmware data: {}", e))?
        .to_vec();

    // Verify SHA-256 integrity
    emit_progress(
        &window,
        "verifying",
        35,
        "Verifying firmware integrity (SHA-256)...",
    );
    let mut hasher = Sha256::new();
    hasher.update(&bin_bytes);
    let computed_hash = format!("{:x}", hasher.finalize());

    if let Some(ref expected) = expected_sha256 {
        let expected_clean = expected.trim().to_lowercase();
        if computed_hash != expected_clean {
            let msg = format!(
                "Security check failed: firmware SHA-256 mismatch (expected {}, got {}). Aborting update.",
                expected_clean, computed_hash
            );
            emit_progress(&window, "error", 0, &msg);
            return Err(msg);
        }
    }

    emit_progress(
        &window,
        "uploading",
        45,
        "Uploading verified firmware to CubiPal over Wi-Fi...",
    );
    flash_firmware_bytes(&window, &ip, &token, bin_bytes).await
}

#[tauri::command]
pub async fn install_local_firmware(
    window: tauri::Window,
    file_path: String,
    force: Option<bool>,
) -> Result<(), String> {
    emit_progress(&window, "downloading", 10, "Reading local firmware file...");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let (ip, token) = crate::device::get_saved_device_credentials()
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    if let Err(err_msg) = check_device_idle(&client, &ip, &token, force.unwrap_or(false)).await {
        emit_progress(&window, "error", 0, &err_msg);
        return Err(err_msg);
    }

    let bin_bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("Failed to read file '{}': {}", file_path, e))?;

    emit_progress(
        &window,
        "uploading",
        40,
        "Uploading to CubiPal over Wi-Fi...",
    );
    flash_firmware_bytes(&window, &ip, &token, bin_bytes).await
}

async fn flash_firmware_bytes(
    window: &tauri::Window,
    ip: &str,
    token: &str,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("firmware.bin")
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new().part("fw", part);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    emit_progress(
        window,
        "uploading",
        70,
        "Writing firmware to CubiPal flash slot...",
    );

    let res = client
        .post(format!("http://{}/api/update", ip))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Pet", "1")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {}", e))?;

    if res.status().is_success() {
        emit_progress(
            window,
            "rebooting",
            95,
            "Flashing complete. CubiPal is rebooting...",
        );
        tokio::time::sleep(Duration::from_millis(2000)).await;
        emit_progress(
            window,
            "completed",
            100,
            "Update successful! CubiPal has restarted.",
        );
        Ok(())
    } else {
        let msg = if res.status().as_u16() == 409 {
            "Device rejected update: a timer is active on CubiPal.".to_string()
        } else {
            format!(
                "Device rejected update (HTTP {}). Working firmware is untouched.",
                res.status()
            )
        };
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
