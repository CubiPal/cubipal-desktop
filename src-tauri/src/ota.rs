use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Emitter;

/// Firmware releases are published here (see the firmware repo's release workflow).
const FIRMWARE_RELEASES_URL: &str = "https://api.github.com/repos/CubiPal/CubiPal/releases/latest";

/// How long a successful GitHub release lookup is reused. The unauthenticated
/// GitHub API allows 60 requests per hour per IP, so never poll it on a timer.
const RELEASE_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
/// How long a failed lookup is remembered before trying GitHub again.
const RELEASE_FAILURE_TTL: Duration = Duration::from_secs(10 * 60);

/// Every ESP32 application image starts with this magic byte. The merged
/// factory image (bootloader at 0x1000) starts with 0xFF padding instead.
const ESP_IMAGE_MAGIC: u8 = 0xE9;

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
struct FirmwareManifest {
    version: Option<String>,
    firmware_file: Option<String>,
    sha256: Option<String>,
}

/// The OTA image the backend resolved from its own GitHub lookup. The webview
/// never supplies the download URL or hash.
#[derive(Debug, Clone)]
struct ResolvedRelease {
    version: String,
    bin_url: String,
    sha256: Option<String>,
    notes: Option<String>,
}

static RELEASE_CACHE: Mutex<Option<(Instant, Option<ResolvedRelease>)>> = Mutex::new(None);

fn github_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("CubiPal-Desktop")
        .build()
        .map_err(|e| e.to_string())
}

/// Returns the latest firmware release, from cache unless it is stale or
/// `force_refresh` is set.
async fn latest_release(force_refresh: bool) -> Result<Option<ResolvedRelease>, String> {
    if !force_refresh
        && let Ok(guard) = RELEASE_CACHE.lock()
        && let Some((fetched_at, cached)) = guard.as_ref()
    {
        let ttl = if cached.is_some() {
            RELEASE_CACHE_TTL
        } else {
            RELEASE_FAILURE_TTL
        };
        if fetched_at.elapsed() < ttl {
            return Ok(cached.clone());
        }
    }

    let client = github_client()?;
    let resolved = fetch_latest_release(&client).await.ok();
    if let Ok(mut guard) = RELEASE_CACHE.lock() {
        *guard = Some((Instant::now(), resolved.clone()));
    }
    Ok(resolved)
}

async fn fetch_latest_release(client: &reqwest::Client) -> Result<ResolvedRelease, String> {
    let release: GithubRelease = client
        .get(FIRMWARE_RELEASES_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let mut manifest: Option<FirmwareManifest> = None;
    if let Some(asset) = find_manifest_asset(&release.assets)
        && let Ok(resp) = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
    {
        manifest = resp.json::<FirmwareManifest>().await.ok();
    }

    let manifest_file = manifest.as_ref().and_then(|m| m.firmware_file.as_deref());
    let bin = select_firmware_asset(&release.assets, manifest_file)
        .ok_or_else(|| "Release has no OTA firmware image".to_string())?;

    // The manifest hash belongs to the manifest's firmware_file; only trust it
    // for the image we actually picked.
    let mut sha256 = manifest
        .as_ref()
        .filter(|m| {
            m.firmware_file
                .as_deref()
                .is_none_or(|f| f.eq_ignore_ascii_case(&bin.name))
        })
        .and_then(|m| m.sha256.as_deref())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| is_sha256_hex(s));

    if sha256.is_none()
        && let Some(asset) = select_sha_asset(&release.assets, &bin.name)
        && let Ok(resp) = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        && let Ok(text) = resp.text().await
    {
        sha256 = parse_sha256_text(&text, &bin.name);
    }

    let version = manifest
        .as_ref()
        .and_then(|m| m.version.clone())
        .unwrap_or(release.tag_name);

    Ok(ResolvedRelease {
        version: version.trim().trim_start_matches('v').to_string(),
        bin_url: bin.browser_download_url.clone(),
        sha256,
        notes: release.body,
    })
}

/// `force` re-queries GitHub (manual "check now"); otherwise a cached lookup
/// up to an hour old is reused.
#[tauri::command]
pub async fn check_firmware_update(force: Option<bool>) -> Result<FirmwareUpdateInfo, String> {
    let dev = crate::device::get_paired_device()
        .await
        .map_err(|e| format!("Failed to read paired device: {}", e))?
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    let current_version = dev.fw.unwrap_or_else(|| "1.0.0".to_string());

    let Some(release) = latest_release(force.unwrap_or(false)).await? else {
        return Ok(FirmwareUpdateInfo {
            available: false,
            latest_version: current_version.clone(),
            current_version,
            download_url: None,
            sha256: None,
            release_notes: None,
        });
    };

    Ok(FirmwareUpdateInfo {
        available: is_version_newer(&release.version, &current_version),
        latest_version: release.version,
        current_version,
        download_url: Some(release.bin_url),
        sha256: release.sha256,
        release_notes: release.notes,
    })
}

fn find_manifest_asset(assets: &[GithubAsset]) -> Option<&GithubAsset> {
    assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n == "manifest.json" || n.ends_with("-manifest.json")
    })
}

/// True for a `.bin` that can be sent to `/api/update`: never the merged
/// factory image (bootloader + partition table + app).
fn is_ota_image_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".bin") && !n.contains("factory") && !n.contains("merged")
}

/// Picks the OTA app image: the manifest's `firmware_file` when it names an
/// OTA image in the release, else a `*firmware*.bin`, else any other OTA `.bin`.
fn select_firmware_asset<'a>(
    assets: &'a [GithubAsset],
    manifest_file: Option<&str>,
) -> Option<&'a GithubAsset> {
    if let Some(wanted) = manifest_file
        && let Some(a) = assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(wanted) && is_ota_image_name(&a.name))
    {
        return Some(a);
    }
    assets
        .iter()
        .filter(|a| is_ota_image_name(&a.name))
        .find(|a| a.name.to_ascii_lowercase().contains("firmware"))
        .or_else(|| assets.iter().find(|a| is_ota_image_name(&a.name)))
}

/// Prefers a per-file checksum (`<bin>.sha256`), else a combined
/// `sha256sums.txt`-style file.
fn select_sha_asset<'a>(assets: &'a [GithubAsset], bin_name: &str) -> Option<&'a GithubAsset> {
    let bin = bin_name.to_ascii_lowercase();
    let stem = bin.trim_end_matches(".bin");
    assets
        .iter()
        .find(|a| {
            let n = a.name.to_ascii_lowercase();
            n == format!("{bin}.sha256") || n == format!("{stem}.sha256")
        })
        .or_else(|| {
            assets.iter().find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.contains("sha256") && !n.ends_with(".bin")
            })
        })
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Reads a checksum file. A line naming another file (e.g. the factory image
/// in `sha256sums.txt`) is ignored; a bare hash line is accepted.
fn parse_sha256_text(text: &str, bin_name: &str) -> Option<String> {
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        let Some(hash) = tokens.next() else { continue };
        if !is_sha256_hex(hash) {
            continue;
        }
        match tokens.next() {
            None => return Some(hash.to_ascii_lowercase()),
            Some(file) if file.trim_start_matches('*').eq_ignore_ascii_case(bin_name) => {
                return Some(hash.to_ascii_lowercase());
            }
            Some(_) => {}
        }
    }
    None
}

/// Rejects anything that is not an ESP32 app image (HTML error pages, the
/// merged factory image, random files) before it is sent to the Pal.
fn check_esp_image(bytes: &[u8]) -> Result<(), String> {
    match bytes.first() {
        Some(&ESP_IMAGE_MAGIC) => Ok(()),
        Some(_) => Err(
            "This file is not a CubiPal OTA firmware image (wrong header). Use the CubiPal-firmware-*.bin file, not the factory image."
                .to_string(),
        ),
        None => Err("The firmware file is empty.".to_string()),
    }
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

const IDLE_UNKNOWN_MSG: &str = "Couldn't confirm your Pal is idle, so the update was not started. Make sure your Pal is online and not in a meeting or focus timer, then try again.";

// Ensure the device is not in the middle of a meeting or timer before updating (KAN-26 UX requirement).
// Fails closed: if the Pal's state can't be read, the update does not start.
async fn check_device_idle(
    client: &reqwest::Client,
    ip: &str,
    token: &str,
    force: bool,
) -> Result<(), String> {
    if force {
        return Ok(());
    }

    let state = client
        .get(format!("http://{}/api/state", ip))
        .header("X-Pet", "1")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| IDLE_UNKNOWN_MSG.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|_| IDLE_UNKNOWN_MSG.to_string())?;

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
        if start_epoch > 0 && (start_epoch - now_epoch) <= 300 && (now_epoch - start_epoch) < 3600 {
            let title = next_event["title"].as_str().unwrap_or("Meeting");
            return Err(format!(
                "Cannot install update: calendar meeting '{}' is starting or in progress. Updates are paused to prevent desk interruptions.",
                title
            ));
        }
    }
    Ok(())
}

/// Installs the latest firmware release. The download URL and expected hash
/// come from the backend's own release lookup, never from the webview.
#[tauri::command]
pub async fn install_firmware_update(
    window: tauri::Window,
    force: Option<bool>,
) -> Result<(), String> {
    let fail = |msg: String| {
        emit_progress(&window, "error", 0, &msg);
        Err(msg)
    };

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
        return fail(err_msg);
    }

    let Some(release) = latest_release(false).await? else {
        return fail("Couldn't find the latest firmware release on GitHub. Check your internet connection and try again.".to_string());
    };

    let download = client
        .get(&release.bin_url)
        .send()
        .await
        .and_then(|r| r.error_for_status());
    let bin_bytes = match download {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return fail(format!("Failed to read firmware data: {}", e)),
        },
        Err(e) => return fail(format!("Failed to download firmware: {}", e)),
    };

    if let Err(msg) = check_esp_image(&bin_bytes) {
        return fail(msg);
    }

    // Verify SHA-256 integrity when the release publishes a checksum
    let verified = match release.sha256 {
        Some(ref expected) => {
            emit_progress(
                &window,
                "verifying",
                35,
                "Verifying firmware integrity (SHA-256)...",
            );
            let computed_hash = format!("{:x}", Sha256::digest(&bin_bytes));
            if computed_hash != *expected {
                return fail(format!(
                    "Security check failed: firmware SHA-256 mismatch (expected {}, got {}). Aborting update.",
                    expected, computed_hash
                ));
            }
            true
        }
        None => false,
    };

    emit_progress(
        &window,
        "uploading",
        45,
        if verified {
            "Uploading verified firmware to CubiPal over Wi-Fi..."
        } else {
            "No checksum was published for this release, so the download could not be verified. Uploading to CubiPal over Wi-Fi..."
        },
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

    if let Err(msg) = check_esp_image(&bin_bytes) {
        emit_progress(&window, "error", 0, &msg);
        return Err(msg);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GithubAsset {
        GithubAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    fn release_assets(factory_first: bool) -> Vec<GithubAsset> {
        let mut v = vec![
            asset("CubiPal-firmware-v1.2.0.bin"),
            asset("CubiPal-factory-v1.2.0.bin"),
            asset("manifest.json"),
            asset("sha256sums.txt"),
        ];
        if factory_first {
            v.swap(0, 1);
        }
        v
    }

    #[test]
    fn selects_ota_image_never_factory_in_either_order() {
        for factory_first in [false, true] {
            let assets = release_assets(factory_first);
            let picked = select_firmware_asset(&assets, None).unwrap();
            assert_eq!(picked.name, "CubiPal-firmware-v1.2.0.bin");
            let picked = select_firmware_asset(&assets, Some("CubiPal-firmware-v1.2.0.bin"));
            assert_eq!(picked.unwrap().name, "CubiPal-firmware-v1.2.0.bin");
        }
    }

    #[test]
    fn manifest_cannot_point_at_factory_or_merged_image() {
        let assets = vec![
            asset("CubiPal-merged-v1.bin"),
            asset("CubiPal-factory-v1.bin"),
            asset("CubiPal-firmware-v1.bin"),
        ];
        let picked = select_firmware_asset(&assets, Some("CubiPal-factory-v1.bin")).unwrap();
        assert_eq!(picked.name, "CubiPal-firmware-v1.bin");
    }

    #[test]
    fn manifest_named_file_is_preferred() {
        let assets = vec![asset("CubiPal-firmware-old.bin"), asset("app-s3.BIN")];
        let picked = select_firmware_asset(&assets, Some("app-s3.bin")).unwrap();
        assert_eq!(picked.name, "app-s3.BIN");
    }

    #[test]
    fn no_ota_image_gives_none() {
        let assets = vec![asset("CubiPal-factory-v1.bin"), asset("manifest.json")];
        assert!(select_firmware_asset(&assets, None).is_none());
        assert!(select_firmware_asset(&[], None).is_none());
    }

    #[test]
    fn finds_manifest_asset() {
        assert!(find_manifest_asset(&[asset("foo-manifest.json")]).is_some());
        assert!(find_manifest_asset(&[asset("Manifest.JSON")]).is_some());
        assert!(find_manifest_asset(&[asset("manifest.json.sig")]).is_none());
    }

    #[test]
    fn selects_matching_sha_asset() {
        let assets = vec![
            asset("CubiPal-factory-v1.bin.sha256"),
            asset("CubiPal-firmware-v1.bin.sha256"),
        ];
        let picked = select_sha_asset(&assets, "CubiPal-firmware-v1.bin").unwrap();
        assert_eq!(picked.name, "CubiPal-firmware-v1.bin.sha256");
        let assets = release_assets(false);
        let picked = select_sha_asset(&assets, "CubiPal-firmware-v1.2.0.bin").unwrap();
        assert_eq!(picked.name, "sha256sums.txt");
    }

    #[test]
    fn parses_sha256sums_for_the_right_file() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let text = format!("{b}  CubiPal-factory-v1.2.0.bin\n{a}  CubiPal-firmware-v1.2.0.bin\n");
        assert_eq!(
            parse_sha256_text(&text, "CubiPal-firmware-v1.2.0.bin"),
            Some(a.clone())
        );
        assert_eq!(
            parse_sha256_text(&text, "CubiPal-factory-v1.2.0.bin"),
            Some(b.to_lowercase())
        );
        // binary-mode marker and bare hash
        let text = format!("{a} *CubiPal-firmware-v1.2.0.bin");
        assert_eq!(
            parse_sha256_text(&text, "CubiPal-firmware-v1.2.0.bin"),
            Some(a.clone())
        );
        assert_eq!(
            parse_sha256_text(&format!("{b}\n"), "x.bin"),
            Some(b.to_lowercase())
        );
        // wrong lengths and other files are rejected
        assert_eq!(parse_sha256_text(&"a".repeat(63), "x.bin"), None);
        assert_eq!(parse_sha256_text(&"a".repeat(65), "x.bin"), None);
        assert_eq!(parse_sha256_text(&format!("{a}  other.bin"), "x.bin"), None);
        assert_eq!(parse_sha256_text("", "x.bin"), None);
    }

    #[test]
    fn manifest_from_release_workflow_deserializes() {
        let json = r#"{
            "version": "1.2.0",
            "tag": "v1.2.0",
            "firmware_file": "CubiPal-firmware-v1.2.0.bin",
            "factory_file": "CubiPal-factory-v1.2.0.bin",
            "sha256": "abc",
            "min_desktop_version": "0.1.0",
            "release_date": "2026-11-12"
        }"#;
        let m: FirmwareManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.version.as_deref(), Some("1.2.0"));
        assert_eq!(
            m.firmware_file.as_deref(),
            Some("CubiPal-firmware-v1.2.0.bin")
        );
        assert_eq!(m.sha256.as_deref(), Some("abc"));
    }

    #[test]
    fn esp_image_magic_is_checked() {
        assert!(check_esp_image(&[0xE9, 0x03, 0x02]).is_ok());
        assert!(check_esp_image(&[0xFF, 0xFF]).is_err());
        assert!(check_esp_image(b"<!DOCTYPE html>").is_err());
        assert!(check_esp_image(&[]).is_err());
    }

    #[test]
    fn version_compare() {
        assert!(is_version_newer("1.0.1", "1.0.0"));
        assert!(!is_version_newer("1.0.0", "1.0.0"));
        assert!(is_version_newer("1.10.0", "1.9.9"));
        assert!(!is_version_newer("1.9.9", "1.10.0"));
        assert!(is_version_newer("2.0.0", "1.9.0"));
        assert!(!is_version_newer("", "1.0.0"));
    }
}
