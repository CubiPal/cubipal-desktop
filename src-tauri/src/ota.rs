use semver::{BuildMetadata, Version};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Emitter;

/// Firmware releases are published here by the firmware CI (contract §3).
/// The app never uses the GitHub API for firmware.
const LATEST_JSON_URL: &str = "https://cubipal.com/firmware/latest.json";
/// Firmware images (and release notes) are only ever fetched from
/// `https://cubipal.com/firmware/...`.
const FIRMWARE_HOST: &str = "cubipal.com";
const FIRMWARE_PATH_PREFIX: &str = "/firmware/";

/// How long a fetched `latest.json` is reused.
const LATEST_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
/// How long a failed fetch is remembered before trying again.
const LATEST_FAILURE_TTL: Duration = Duration::from_secs(10 * 60);

/// Largest image the app will download (the Pal's flash is 4 MB).
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Every ESP32 image (app or bootloader) starts with this magic byte.
const ESP_IMAGE_MAGIC: u8 = 0xE9;
/// `ESP_APP_DESC_MAGIC_WORD`: an application image has its `esp_app_desc_t`
/// right after the 24-byte image header and the 8-byte first segment header.
/// A bootloader (and so the S3 merged factory image, which starts with the
/// bootloader at offset 0) has something else there.
const ESP_APP_DESC_MAGIC: u32 = 0xABCD_5432;
const ESP_APP_DESC_OFFSET: usize = 0x20;

/// Chip ids this app knows how to update (contract §2), with the chip id the
/// ESP-IDF image header (`esp_image_header_t.chip_id`, bytes 12-13) carries.
const KNOWN_CHIPS: &[(&str, u16)] = &[("esp32", 0x0000), ("esp32s3", 0x0009)];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareUpdateInfo {
    /// A newer release exists for this Pal.
    pub available: bool,
    pub latest_version: Option<String>,
    /// The Pal's version for display ("v1.1.0" or "unknown version").
    pub current_version: String,
    pub chip: Option<String>,
    /// Set when an update exists (or might) but can't be installed from the
    /// app, or the check couldn't run. Shown to the user as is.
    pub blocked_reason: Option<String>,
    pub notes_url: Option<String>,
    /// The Pal can't be updated over Wi-Fi (its firmware doesn't report its
    /// board); it needs one USB install from this page first.
    pub recover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaProgress {
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Versions (contract §1)
// ---------------------------------------------------------------------------

/// Parses a firmware version as semver (`v` prefix allowed). Build metadata
/// (`+3.gabc123`) is dropped, so a dev build equals its release. Legacy
/// strings like `"1 Nov 12 2026"` give `None`.
pub fn parse_version(raw: &str) -> Option<Version> {
    let t = raw.trim();
    let t = t
        .strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t);
    let mut v = Version::parse(t).ok()?;
    v.build = BuildMetadata::EMPTY;
    Some(v)
}

/// True when `latest` should be offered to a Pal running `current`. A
/// prerelease ranks below its release; an unparseable or missing current
/// version is older than any release.
pub fn is_update_available(latest: &str, current: Option<&str>) -> bool {
    let Some(latest) = parse_version(latest) else {
        return false;
    };
    match current.and_then(parse_version) {
        Some(current) => latest > current,
        None => true,
    }
}

/// The Pal's version as shown to people.
pub fn version_label(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|s| parse_version(s).is_some()) {
        Some(v) => format!("v{}", v.trim_start_matches(['v', 'V'])),
        None => "unknown version".to_string(),
    }
}

// ---------------------------------------------------------------------------
// latest.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct LatestRelease {
    version: String,
    #[serde(default)]
    notes_url: Option<String>,
    #[serde(default)]
    /// Each chip's entry is parsed on its own when needed, so one malformed
    /// entry doesn't block updates for the other board (B-15).
    builds: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChipBuilds {
    ota: Option<BuildFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct BuildFile {
    url: String,
    sha256: String,
    size: u64,
}

/// The OTA image for one chip, checked to be well-formed. Kept in the
/// backend: the webview never supplies the URL or hash.
#[derive(Debug, Clone, PartialEq)]
struct OtaBuild {
    version: String,
    url: String,
    sha256: String,
    size: u64,
}

type CachedLatest = (Instant, Result<Arc<LatestRelease>, String>);
static LATEST_CACHE: Mutex<Option<CachedLatest>> = Mutex::new(None);

fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("CubiPal-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// `latest.json`, from cache unless it is stale or `force_refresh` is set.
async fn latest_release(force_refresh: bool) -> Result<Arc<LatestRelease>, String> {
    if !force_refresh
        && let Ok(guard) = LATEST_CACHE.lock()
        && let Some((fetched_at, cached)) = guard.as_ref()
    {
        let ttl = if cached.is_ok() {
            LATEST_CACHE_TTL
        } else {
            LATEST_FAILURE_TTL
        };
        if fetched_at.elapsed() < ttl {
            return cached.clone();
        }
    }

    let fetched = fetch_latest_release().await.map(Arc::new);
    if let Ok(mut guard) = LATEST_CACHE.lock() {
        *guard = Some((Instant::now(), fetched.clone()));
    }
    fetched
}

async fn fetch_latest_release() -> Result<LatestRelease, String> {
    let client = http_client(Duration::from_secs(10))?;
    let text = client
        .get(LATEST_JSON_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| match e.status() {
            Some(status) => format!(
                "cubipal.com couldn't provide the firmware release list right now (error {}). Try again later.",
                status.as_u16()
            ),
            None => "Couldn't reach cubipal.com to check for firmware updates. Check your internet connection.".to_string(),
        })?
        .text()
        .await
        .map_err(|e| format!("Couldn't read the firmware release list: {}", e))?;
    parse_latest_json(&text)
}

fn parse_latest_json(text: &str) -> Result<LatestRelease, String> {
    let release: LatestRelease = serde_json::from_str(text)
        .map_err(|e| format!("The firmware release list is malformed: {}", e))?;
    if parse_version(&release.version).is_none() {
        return Err(format!(
            "The firmware release list has an invalid version ({}).",
            release.version
        ));
    }
    Ok(release)
}

fn is_known_chip(chip: &str) -> bool {
    KNOWN_CHIPS.iter().any(|(c, _)| *c == chip)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The URL, normalised, if it is `https://cubipal.com/firmware/...` once
/// parsed (B-6): https only, exactly that host, no port or user info, and
/// the path still under `/firmware/` after `..` and `%2e%2e` segments are
/// resolved.
fn firmware_url(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw.trim()).ok()?;
    let ok = url.scheme() == "https"
        && url.host_str() == Some(FIRMWARE_HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with(FIRMWARE_PATH_PREFIX);
    ok.then(|| url.to_string())
}

/// The OTA build for `chip`, with its URL, hash and size checked.
fn select_build(release: &LatestRelease, chip: &str) -> Result<OtaBuild, String> {
    let missing = || {
        format!(
            "Firmware v{} has no update for this Pal's board ({}).",
            release.version, chip
        )
    };
    let entry = release.builds.get(chip).ok_or_else(missing)?;
    let builds: ChipBuilds = serde_json::from_value(entry.clone()).map_err(|_| {
        format!(
            "The firmware release list has a malformed entry for this Pal's board ({}).",
            chip
        )
    })?;
    let file = builds.ota.ok_or_else(missing)?;
    let Some(url) = firmware_url(&file.url) else {
        return Err("The firmware release list points outside cubipal.com/firmware.".to_string());
    };
    let sha256 = file.sha256.trim().to_ascii_lowercase();
    if !is_sha256_hex(&sha256) {
        return Err("The firmware release list has no valid SHA-256 for this image.".to_string());
    }
    if file.size == 0 || file.size > MAX_IMAGE_BYTES {
        return Err("The firmware release list has an invalid image size.".to_string());
    }
    Ok(OtaBuild {
        version: release
            .version
            .trim()
            .trim_start_matches(['v', 'V'])
            .to_string(),
        url,
        sha256,
        size: file.size,
    })
}

// ---------------------------------------------------------------------------
// Update check
// ---------------------------------------------------------------------------

/// The browser installer for a one-time USB update (TR2-05).
const RECOVER_URL: &str = "https://cubipal.com/recover";
const UNKNOWN_CHIP_MSG: &str = "Your Pal's firmware is too old to tell the app which board it has, so this first update has to be installed over USB from the recovery page. That resets your Pal: afterwards, set up its Wi-Fi again and pair this app again. After that, updates come over Wi-Fi.";

/// What the app would install, or why it can't.
struct UpdatePlan {
    info: FirmwareUpdateInfo,
    build: Option<OtaBuild>,
}

async fn plan_update(force_refresh: bool) -> Result<UpdatePlan, String> {
    let dev = crate::device::get_paired_device()
        .await?
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    let mut info = FirmwareUpdateInfo {
        available: false,
        latest_version: None,
        current_version: dev.fw_label.clone(),
        chip: dev.chip.clone(),
        blocked_reason: None,
        notes_url: None,
        recover_url: None,
    };

    // Offline: the version is unknown, so offer nothing (QA-09).
    if !dev.connected {
        info.current_version = "unknown version".to_string();
        info.blocked_reason = Some("Your Pal is offline.".to_string());
        return Ok(UpdatePlan { info, build: None });
    }

    let release = match latest_release(force_refresh).await {
        Ok(r) => r,
        Err(e) => {
            info.blocked_reason = Some(e);
            return Ok(UpdatePlan { info, build: None });
        }
    };
    info.latest_version = Some(release.version.trim_start_matches(['v', 'V']).to_string());
    info.notes_url = release.notes_url.clone().as_deref().and_then(firmware_url);
    info.available = is_update_available(&release.version, dev.fw.as_deref());
    if !info.available {
        return Ok(UpdatePlan { info, build: None });
    }

    // Never guess the board: a classic image can't run on an S3 and vice versa.
    let chip = match dev.chip.as_deref() {
        Some(c) if is_known_chip(c) => c.to_string(),
        _ => {
            info.blocked_reason = Some(UNKNOWN_CHIP_MSG.to_string());
            info.recover_url = Some(RECOVER_URL.to_string());
            return Ok(UpdatePlan { info, build: None });
        }
    };
    match select_build(&release, &chip) {
        Ok(build) => Ok(UpdatePlan {
            info,
            build: Some(build),
        }),
        Err(e) => {
            info.blocked_reason = Some(e);
            Ok(UpdatePlan { info, build: None })
        }
    }
}

/// `force` re-fetches `latest.json` (manual "check now"); otherwise a copy up
/// to an hour old is reused.
#[tauri::command]
pub async fn check_firmware_update(force: Option<bool>) -> Result<FirmwareUpdateInfo, String> {
    plan_update(force.unwrap_or(false)).await.map(|p| p.info)
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Rejects anything that is not an ESP32 app image for this chip (HTML error
/// pages, the merged factory image, the other board's image).
fn check_esp_image(bytes: &[u8], chip: Option<&str>) -> Result<(), String> {
    match bytes.first() {
        Some(&ESP_IMAGE_MAGIC) => {}
        Some(_) => {
            return Err(
                "This file is not a CubiPal OTA firmware image (wrong header). It may be the factory image, which is for USB flashing only."
                    .to_string(),
            );
        }
        None => return Err("The firmware file is empty.".to_string()),
    }
    let app_magic = bytes
        .get(ESP_APP_DESC_OFFSET..ESP_APP_DESC_OFFSET + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if app_magic != Some(ESP_APP_DESC_MAGIC) {
        return Err(
            "This file is not a CubiPal OTA firmware image (no app descriptor). It may be the factory or bootloader image, which is for USB flashing only."
                .to_string(),
        );
    }
    if let Some(chip) = chip
        && let Some((_, want)) = KNOWN_CHIPS.iter().find(|(c, _)| *c == chip)
    {
        let Some(id_bytes) = bytes.get(12..14) else {
            return Err("The firmware image is truncated.".to_string());
        };
        let got = u16::from_le_bytes([id_bytes[0], id_bytes[1]]);
        if got != *want {
            return Err(format!(
                "This firmware image is for a different board (chip id {:#06x}), not this Pal ({}).",
                got, chip
            ));
        }
    }
    Ok(())
}

/// Checks a downloaded image against the release list: exact size, SHA-256
/// (always required), then the image header.
fn verify_image(bytes: &[u8], build: &OtaBuild, chip: &str) -> Result<(), String> {
    if bytes.len() as u64 != build.size {
        return Err(format!(
            "The download is {} bytes but should be {} bytes.",
            bytes.len(),
            build.size
        ));
    }
    let computed = format!("{:x}", Sha256::digest(bytes));
    if computed != build.sha256 {
        return Err(format!(
            "SHA-256 mismatch (expected {}, got {}).",
            build.sha256, computed
        ));
    }
    check_esp_image(bytes, Some(chip))
}

async fn download_image(build: &OtaBuild) -> Result<Vec<u8>, String> {
    // No redirects: the image must come from the checked URL itself.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("CubiPal-Desktop/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut res = client
        .get(&build.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.without_url().to_string())?;
    // Don't let a wrong or hostile response fill memory.
    if res.content_length().is_some_and(|len| len > build.size) {
        return Err("the server sent a larger file than expected".to_string());
    }
    let mut bytes = Vec::with_capacity(build.size as usize);
    while let Some(chunk) = res.chunk().await.map_err(|e| e.without_url().to_string())? {
        if bytes.len() as u64 + chunk.len() as u64 > build.size {
            return Err("the server sent a larger file than expected".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
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
        .bearer_auth(token)
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
        return Err("A Focus or Countdown timer is running on your Pal. Stop the timer or wait for it to finish first.".to_string());
    }

    // Check upcoming or ongoing meeting
    if let Some(next_event) = state.get("nextEvent").filter(|e| !e.is_null()) {
        let start_epoch = next_event["start"].as_i64().unwrap_or(0);
        let now_epoch = chrono::Utc::now().timestamp();
        // Meeting is within 5 minutes or in progress (<1hr past start)
        if start_epoch > 0 && (start_epoch - now_epoch) <= 300 && (now_epoch - start_epoch) < 3600 {
            let title = next_event["title"].as_str().unwrap_or("Meeting");
            return Err(format!(
                "The meeting '{}' is starting or in progress. Updates wait so your desk isn't interrupted.",
                title
            ));
        }
    }
    Ok(())
}

/// Installs the latest firmware for the paired Pal's board. The download URL
/// and expected hash come from the backend's own `latest.json` lookup, never
/// from the webview.
#[tauri::command]
pub async fn install_firmware_update(
    window: tauri::Window,
    force: Option<bool>,
) -> Result<(), String> {
    // Every failure names the stage it happened in (TST-19).
    let fail = |stage: &str, msg: String| {
        let full = format!("{}: {}", stage, msg);
        emit_progress(&window, "error", 0, &full);
        Err(full)
    };

    emit_progress(
        &window,
        "checking",
        5,
        "Checking your Pal and the latest release...",
    );

    let plan = match plan_update(false).await {
        Ok(p) => p,
        Err(e) => return fail("Update check failed", e),
    };
    let Some(build) = plan.build else {
        let reason = plan
            .info
            .blocked_reason
            .unwrap_or_else(|| "Your Pal is already up to date.".to_string());
        return fail("Update not started", reason);
    };
    let chip = plan.info.chip.unwrap_or_default();

    let target = match crate::device::paired_target().await {
        Ok(t) => t,
        Err(e) => return fail("Update not started", e),
    };
    let client = http_client(Duration::from_secs(5))?;
    if let Err(msg) =
        check_device_idle(&client, &target.ip, &target.token, force.unwrap_or(false)).await
    {
        return fail("Update not started", msg);
    }

    emit_progress(
        &window,
        "downloading",
        15,
        &format!(
            "Downloading firmware v{} from cubipal.com...",
            build.version
        ),
    );
    let bytes = match download_image(&build).await {
        Ok(b) => b,
        Err(e) => return fail("Download failed", e),
    };

    emit_progress(
        &window,
        "verifying",
        35,
        "Checking the download (size and SHA-256)...",
    );
    if let Err(msg) = verify_image(&bytes, &build, &chip) {
        return fail("Download check failed, nothing was sent to your Pal", msg);
    }

    emit_progress(
        &window,
        "uploading",
        45,
        "Uploading verified firmware to your Pal over Wi-Fi...",
    );
    match flash_firmware_bytes(&window, &target, bytes).await {
        Ok(()) => Ok(()),
        Err(e) => fail("Upload to your Pal failed", e),
    }
}

async fn flash_firmware_bytes(
    window: &tauri::Window,
    target: &crate::device::Target,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("firmware.bin")
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;

    let form = reqwest::multipart::Form::new().part("fw", part);

    let client = http_client(Duration::from_secs(90))?;

    emit_progress(
        window,
        "uploading",
        70,
        "Writing firmware to your Pal's spare flash slot...",
    );

    let res = client
        .post(format!("http://{}/api/update", target.ip))
        .bearer_auth(&target.token)
        .header("X-Pet", "1")
        .multipart(form)
        .send()
        .await
        .map_err(crate::device::reach_error)?;

    if res.status().is_success() {
        emit_progress(
            window,
            "rebooting",
            95,
            "Flashing complete. Your Pal is restarting...",
        );
        tokio::time::sleep(Duration::from_millis(2000)).await;
        emit_progress(
            window,
            "completed",
            100,
            "Firmware sent. Your Pal is restarting with the new version...",
        );
        Ok(())
    } else {
        // The Pal explains itself in {"ok":false,"msg":...}; its working
        // firmware is untouched when it refuses.
        Err(format!(
            "{} Its current firmware is untouched.",
            crate::device::device_error(res).await
        ))
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

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn latest(json_builds: &str) -> LatestRelease {
        parse_latest_json(&format!(
            r#"{{"version":"1.1.0","released":"2026-11-12T00:00:00Z","notes_url":"https://cubipal.com/firmware/1.1.0/NOTES.md","builds":{json_builds}}}"#
        ))
        .unwrap()
    }

    fn contract_release() -> LatestRelease {
        latest(&format!(
            r#"{{
              "esp32s3": {{
                "ota":     {{ "url": "https://cubipal.com/firmware/1.1.0/cubipal-esp32s3-1.1.0.bin", "sha256": "{SHA_A}", "size": 1178515 }},
                "factory": {{ "url": "https://cubipal.com/firmware/1.1.0/cubipal-esp32s3-1.1.0-factory.bin", "sha256": "{SHA_A}", "size": 0 }}
              }},
              "esp32": {{
                "ota":     {{ "url": "https://cubipal.com/firmware/1.1.0/cubipal-esp32-1.1.0.bin", "sha256": "{}", "size": 1217141 }}
              }}
            }}"#,
            SHA_A.to_uppercase()
        ))
    }

    #[test]
    fn picks_the_ota_build_for_the_chip() {
        let r = contract_release();
        let s3 = select_build(&r, "esp32s3").unwrap();
        assert_eq!(
            s3.url,
            "https://cubipal.com/firmware/1.1.0/cubipal-esp32s3-1.1.0.bin"
        );
        assert_eq!(s3.size, 1_178_515);
        assert_eq!(s3.version, "1.1.0");
        let classic = select_build(&r, "esp32").unwrap();
        assert!(classic.url.ends_with("cubipal-esp32-1.1.0.bin"));
        assert_eq!(classic.sha256, SHA_A, "hash is normalised to lowercase");
        assert!(select_build(&r, "esp32c3").is_err());
    }

    #[test]
    fn rejects_bad_build_entries() {
        let entry = |url: &str, sha: &str, size: u64| {
            latest(&format!(
                r#"{{"esp32":{{"ota":{{"url":"{url}","sha256":"{sha}","size":{size}}}}}}}"#
            ))
        };
        let ok = "https://cubipal.com/firmware/1.1.0/cubipal-esp32-1.1.0.bin";
        assert!(select_build(&entry(ok, SHA_A, 10), "esp32").is_ok());
        assert!(select_build(&entry("https://evil.example/x.bin", SHA_A, 10), "esp32").is_err());
        assert!(
            select_build(
                &entry("https://cubipal.com/firmware/../x.bin", SHA_A, 10),
                "esp32"
            )
            .is_err()
        );
        assert!(
            select_build(
                &entry("http://cubipal.com/firmware/x.bin", SHA_A, 10),
                "esp32"
            )
            .is_err()
        );
        assert!(select_build(&entry(ok, "", 10), "esp32").is_err());
        assert!(select_build(&entry(ok, &SHA_A[..63], 10), "esp32").is_err());
        assert!(select_build(&entry(ok, SHA_A, 0), "esp32").is_err());
        assert!(select_build(&entry(ok, SHA_A, MAX_IMAGE_BYTES + 1), "esp32").is_err());
        // URLs are checked after parsing, not as strings (B-6).
        for bad in [
            "https://cubipal.com/firmware/%2e%2e/x.bin",
            "https://cubipal.com/firmware/../x.bin",
            "https://cubipal.com/firmwarex/x.bin",
            "https://cubipal.com:8443/firmware/x.bin",
            "https://user@cubipal.com/firmware/x.bin",
            "https://cubipal.com.evil.io/firmware/x.bin",
            "https://evil.io/https://cubipal.com/firmware/x.bin",
            "ftp://cubipal.com/firmware/x.bin",
            "not a url",
        ] {
            assert!(
                select_build(&entry(bad, SHA_A, 10), "esp32").is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            select_build(
                &entry("https://CubiPal.com/firmware/1.1.0/a.bin", SHA_A, 10),
                "esp32"
            )
            .unwrap()
            .url,
            "https://cubipal.com/firmware/1.1.0/a.bin"
        );
        // An entry with only a factory image has no OTA build.
        let r = latest(&format!(
            r#"{{"esp32":{{"factory":{{"url":"{ok}","sha256":"{SHA_A}","size":10}}}}}}"#
        ));
        assert!(select_build(&r, "esp32").is_err());
    }

    #[test]
    fn one_bad_chip_entry_does_not_block_the_other() {
        let r = latest(&format!(
            r#"{{
              "esp32": {{ "ota": {{ "url": "https://cubipal.com/firmware/1.1.0/a.bin", "sha256": null, "size": 10 }} }},
              "esp32s3": {{ "ota": {{ "url": "https://cubipal.com/firmware/1.1.0/b.bin", "sha256": "{SHA_A}", "size": 10 }} }},
              "esp32c3": "garbage"
            }}"#
        ));
        assert!(select_build(&r, "esp32s3").is_ok());
        assert!(select_build(&r, "esp32").unwrap_err().contains("malformed"));
        assert!(select_build(&r, "esp32c3").is_err());
    }

    #[test]
    fn latest_json_must_have_a_semver_version() {
        assert!(parse_latest_json(r#"{"version":"1.1.0","builds":{}}"#).is_ok());
        assert!(parse_latest_json(r#"{"version":"v1.2.0-rc.1"}"#).is_ok());
        assert!(parse_latest_json(r#"{"version":"1 Nov 12 2026","builds":{}}"#).is_err());
        assert!(parse_latest_json("<html>").is_err());
    }

    #[test]
    fn semver_comparison_follows_the_contract() {
        assert!(is_update_available("1.0.1", Some("1.0.0")));
        assert!(!is_update_available("1.0.0", Some("1.0.0")));
        assert!(is_update_available("1.10.0", Some("1.9.9")));
        assert!(!is_update_available("1.9.9", Some("1.10.0")));
        assert!(is_update_available("2.0.0", Some("v1.9.0")));
        // A prerelease ranks below its release.
        assert!(is_update_available("1.1.0", Some("1.1.0-rc.1")));
        assert!(!is_update_available("1.1.0-rc.1", Some("1.1.0")));
        assert!(is_update_available("1.1.0-rc.2", Some("1.1.0-rc.1")));
        // Build metadata is ignored: a dev build equals its release.
        assert!(!is_update_available("1.1.0", Some("1.1.0+3.gabc1234")));
        assert!(!is_update_available(
            "1.1.0",
            Some("1.1.0+3.gabc1234.dirty")
        ));
        assert!(is_update_available("1.1.1", Some("1.1.0+3.gabc1234")));
        // Legacy, unknown or missing versions are older than any release.
        assert!(is_update_available("1.1.0", Some("1 Nov 12 2026")));
        assert!(is_update_available("1.1.0", Some("")));
        assert!(is_update_available("1.1.0", None));
        assert!(is_update_available("1.1.0", Some("0.0.0-dev")));
        // A bad "latest" is never offered.
        assert!(!is_update_available("", Some("1.0.0")));
        assert!(!is_update_available("garbage", None));
    }

    #[test]
    fn version_labels() {
        assert_eq!(version_label(Some("1.1.0")), "v1.1.0");
        assert_eq!(version_label(Some("v1.1.0")), "v1.1.0");
        assert_eq!(version_label(Some("1.1.0+3.gabc")), "v1.1.0+3.gabc");
        assert_eq!(version_label(Some("1 Nov 12 2026")), "unknown version");
        assert_eq!(version_label(Some("")), "unknown version");
        assert_eq!(version_label(None), "unknown version");
    }

    fn image(chip_id: u16, len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len.max(0x24)];
        v[0] = 0xE9;
        v[12..14].copy_from_slice(&chip_id.to_le_bytes());
        v[0x20..0x24].copy_from_slice(&ESP_APP_DESC_MAGIC.to_le_bytes());
        v
    }

    /// The first 0x30 bytes of the real v1.1.0 S3 images (QA `xxd`). The
    /// merged factory image starts with the bootloader, which also has `E9`
    /// and chip id 0x0009, but no app descriptor at 0x20.
    const S3_OTA_HEAD: [u8; 0x30] = [
        0xe9, 0x07, 0x02, 0x2f, 0xcc, 0x5c, 0x37, 0x40, 0xee, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00,
        0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x20, 0x00, 0x0e, 0x3c, 0xa4, 0x20,
        0x03, 0x00, 0x32, 0x54, 0xcd, 0xab, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00,
    ];
    const S3_FACTORY_HEAD: [u8; 0x30] = [
        0xe9, 0x03, 0x02, 0x2f, 0xb8, 0x88, 0x3c, 0x40, 0xee, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00,
        0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x20, 0x28, 0xce, 0x3f, 0xcc, 0x10,
        0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x76, 0x35, 0x2e, 0x35, 0x2e,
        0x35, 0x00, 0x00,
    ];

    #[test]
    fn factory_and_bootloader_images_are_refused() {
        assert!(check_esp_image(&S3_OTA_HEAD, Some("esp32s3")).is_ok());
        let err = check_esp_image(&S3_FACTORY_HEAD, Some("esp32s3")).unwrap_err();
        assert!(err.contains("factory"), "{err}");
        assert!(check_esp_image(&S3_FACTORY_HEAD, None).is_err());
        // Too short to hold an app descriptor.
        assert!(check_esp_image(&S3_OTA_HEAD[..0x22], Some("esp32s3")).is_err());
    }

    #[test]
    fn esp_image_header_is_checked() {
        assert!(check_esp_image(&image(0x0009, 64), Some("esp32s3")).is_ok());
        assert!(check_esp_image(&image(0x0000, 64), Some("esp32")).is_ok());
        // The other board's image is refused.
        assert!(check_esp_image(&image(0x0000, 64), Some("esp32s3")).is_err());
        assert!(check_esp_image(&image(0x0009, 64), Some("esp32")).is_err());
        assert!(check_esp_image(&[0xFF, 0xFF], None).is_err());
        assert!(check_esp_image(b"<!DOCTYPE html>", None).is_err());
        assert!(check_esp_image(&[], None).is_err());
        assert!(check_esp_image(&[0xE9, 0x03], Some("esp32")).is_err());
    }

    #[test]
    fn downloads_are_verified_by_size_and_hash() {
        let bytes = image(0x0009, 64);
        let sha = format!("{:x}", Sha256::digest(&bytes));
        let build = OtaBuild {
            version: "1.1.0".into(),
            url: "https://cubipal.com/firmware/1.1.0/x.bin".into(),
            sha256: sha.clone(),
            size: 64,
        };
        assert!(verify_image(&bytes, &build, "esp32s3").is_ok());
        assert!(verify_image(&bytes, &build, "esp32").is_err());
        let wrong_size = OtaBuild {
            size: 65,
            ..build.clone()
        };
        assert!(verify_image(&bytes, &wrong_size, "esp32s3").is_err());
        let wrong_hash = OtaBuild {
            sha256: SHA_A.into(),
            ..build
        };
        assert!(
            verify_image(&bytes, &wrong_hash, "esp32s3")
                .unwrap_err()
                .contains("SHA-256")
        );
    }
}
