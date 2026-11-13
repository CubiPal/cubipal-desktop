use keyring::Entry;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

const KEYRING_SERVICE: &str = "cubipal";
const KEYRING_TOKEN_USER: &str = "device_token";
const KEYRING_IP_USER: &str = "device_ip";
/// The paired Pal's device id (contract §5). Configs saved before 0.2.0 have
/// no id; it is captured on the next successful contact.
const KEYRING_ID_USER: &str = "device_id";

const MDNS_SERVICE: &str = "_cubipal._tcp.local.";
/// How long a scan or a targeted lookup listens for mDNS answers.
const MDNS_BROWSE_TIME: Duration = Duration::from_secs(3);
/// A Pal whose identity was just checked isn't re-checked for this long.
const VERIFIED_TTL: Duration = Duration::from_secs(60);
/// After the paired Pal couldn't be found, wait this long before browsing
/// mDNS again (the UI polls every 15 s).
const RELOCATE_RETRY: Duration = Duration::from_secs(60);

/// Shown when the Pal doesn't answer on the network (TR2-06).
pub const UNREACHABLE_MSG: &str =
    "Couldn't reach your Pal \u{2014} is it on and on the same Wi-Fi?";
/// Shown for HTTP 409: the firmware refuses while a timer or bedtime runs.
pub const BUSY_MSG: &str =
    "Your Pal is busy (a timer or bedtime is running) \u{2014} try again after it ends.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPal {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub model: String,
    pub chip: Option<String>,
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
    /// Raw firmware version string as reported by the Pal.
    pub fw: Option<String>,
    /// Version for display: `v1.1.0`, or "unknown version" for legacy builds.
    pub fw_label: String,
    pub chip: Option<String>,
    pub model: Option<String>,
    pub id: Option<String>,
    pub skin: Option<String>,
    pub ip: String,
    pub connected: bool,
    /// Why the app can't talk to the Pal, when it isn't simply offline.
    pub problem: Option<String>,
    /// The Pal's firmware predates `/api/info`, so its identity can't be
    /// checked before the token is sent (B-4): suggest a firmware update.
    #[serde(default)]
    pub legacy_pairing: bool,
}

impl DeviceState {
    fn unreachable(ip: String, id: Option<String>, problem: Option<String>) -> Self {
        DeviceState {
            robot: "Cubi".to_string(),
            owner: String::new(),
            mood: None,
            volume: None,
            muted: None,
            ssid: None,
            rssi: None,
            fw: None,
            fw_label: "unknown version".to_string(),
            chip: None,
            model: None,
            id,
            skin: None,
            ip,
            connected: false,
            problem,
            legacy_pairing: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResponse {
    pub ok: bool,
    pub token: Option<String>,
    pub robot: Option<String>,
    pub owner: Option<String>,
    pub fw: Option<String>,
    pub id: Option<String>,
    pub chip: Option<String>,
    pub msg: Option<String>,
}

// ---------------------------------------------------------------------------
// Saved pairing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SavedDevice {
    pub ip: String,
    pub token: String,
    pub id: Option<String>,
}

fn keyring_get(user: &str) -> Option<String> {
    Entry::new(KEYRING_SERVICE, user)
        .and_then(|e| e.get_password())
        .ok()
}

fn keyring_set(user: &str, value: &str) -> Result<(), String> {
    Entry::new(KEYRING_SERVICE, user)
        .and_then(|e| e.set_password(value))
        .map_err(|e| e.to_string())
}

fn keyring_delete(user: &str) {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, user) {
        let _ = entry.delete_credential();
    }
}

pub fn load_saved_device() -> Option<SavedDevice> {
    let token = keyring_get(KEYRING_TOKEN_USER)?;
    let ip = keyring_get(KEYRING_IP_USER)?;
    let id = keyring_get(KEYRING_ID_USER)
        .map(|s| normalize_id(&s))
        .filter(|s| is_valid_id(s));
    Some(SavedDevice { ip, token, id })
}

fn store_device(ip: &str, token: &str, id: Option<&str>) -> Result<(), String> {
    keyring_set(KEYRING_TOKEN_USER, token)?;
    keyring_set(KEYRING_IP_USER, ip)?;
    match id {
        Some(id) => keyring_set(KEYRING_ID_USER, id)?,
        // Never leave a previous Pal's id next to a new token.
        None => keyring_delete(KEYRING_ID_USER),
    }
    forget_verification();
    Ok(())
}

pub fn clear_device_credentials() -> Result<(), String> {
    keyring_delete(KEYRING_TOKEN_USER);
    keyring_delete(KEYRING_IP_USER);
    keyring_delete(KEYRING_ID_USER);
    forget_verification();
    Ok(())
}

// ---------------------------------------------------------------------------
// Device ids
// ---------------------------------------------------------------------------

/// Lowercase hex with separators removed (`A0:B1:...` -> `a0b1...`).
pub fn normalize_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | ' '))
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A device id is hex: the full 12-digit MAC id, or the 4-digit suffix that
/// firmware before 1.1.0 advertised over mDNS.
pub fn is_valid_id(id: &str) -> bool {
    (4..=32).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// True when two ids name the same Pal. Older firmware advertised only the
/// last 4 hex digits of the id; a stored 4-digit id matches a full 12-digit
/// id that ends with it (so pairings survive the firmware update), and the
/// caller then upgrades the stored id to the full one.
pub fn ids_match(stored: &str, seen: &str) -> bool {
    let a = normalize_id(stored);
    let b = normalize_id(seen);
    if !is_valid_id(&a) || !is_valid_id(&b) {
        return false;
    }
    a == b
        || (a.len() == 4 && b.len() == 12 && b.ends_with(&a))
        || (b.len() == 4 && a.len() == 12 && a.ends_with(&b))
}

/// A 4-digit id: what firmware before 1.1.0 advertised over mDNS. Such a
/// pairing can't be checked against `/api/info`.
fn is_legacy_id(id: &str) -> bool {
    id.len() == 4
}

/// The better of two matching ids (the full one wins over a legacy suffix).
fn preferred_id(stored: &str, seen: &str) -> String {
    let a = normalize_id(stored);
    let b = normalize_id(seen);
    if b.len() > a.len() { b } else { a }
}

// ---------------------------------------------------------------------------
// mDNS
// ---------------------------------------------------------------------------

/// Browses for Pals for 3 s. The mDNS receiver blocks, so it runs on a
/// blocking thread rather than a tokio worker.
#[tauri::command]
pub async fn scan_for_pals() -> Result<Vec<DiscoveredPal>, String> {
    tokio::task::spawn_blocking(|| browse_blocking(|_| false))
        .await
        .map_err(|e| format!("Scan failed: {}", e))?
}

/// Looks for the Pal with this id on the network; returns its address.
async fn mdns_find_by_id(id: &str) -> Option<DiscoveredPal> {
    let wanted = id.to_string();
    let wanted2 = wanted.clone();
    let pals = tokio::task::spawn_blocking(move || {
        browse_blocking(move |p: &DiscoveredPal| ids_match(&wanted, &p.id))
    })
    .await
    .ok()?
    .ok()?;
    pals.into_iter().find(|p| ids_match(&wanted2, &p.id))
}

/// Looks for whichever Pal answers mDNS at this address (for the chip type
/// of firmware that has no `/api/info`).
pub async fn mdns_find_by_ip(ip: &str) -> Option<DiscoveredPal> {
    let wanted = ip.to_string();
    let wanted2 = wanted.clone();
    let pals = tokio::task::spawn_blocking(move || {
        browse_blocking(move |p: &DiscoveredPal| p.ip == wanted)
    })
    .await
    .ok()?
    .ok()?;
    pals.into_iter().find(|p| p.ip == wanted2)
}

fn browse_blocking(
    stop_when: impl Fn(&DiscoveredPal) -> bool,
) -> Result<Vec<DiscoveredPal>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let result = browse_pals(&mdns, stop_when);
    // Stop the daemon's thread and sockets; one is created per browse.
    let _ = mdns.shutdown();
    result
}

fn browse_pals(
    mdns: &ServiceDaemon,
    stop_when: impl Fn(&DiscoveredPal) -> bool,
) -> Result<Vec<DiscoveredPal>, String> {
    let receiver = mdns.browse(MDNS_SERVICE).map_err(|e| e.to_string())?;

    let mut pals = HashMap::new();
    let start = Instant::now();

    while start.elapsed() < MDNS_BROWSE_TIME {
        if let Ok(event) = receiver.recv_timeout(Duration::from_millis(250))
            && let ServiceEvent::ServiceResolved(info) = event
            && let Some(pal) = pal_from_service(&info)
        {
            let done = stop_when(&pal);
            pals.insert(format!("{}|{}", pal.id, pal.ip), pal);
            if done {
                break;
            }
        }
    }

    let _ = mdns.stop_browse(MDNS_SERVICE);
    Ok(pals.into_values().collect())
}

fn pal_from_service(info: &ServiceInfo) -> Option<DiscoveredPal> {
    let ip = pick_pal_address(info.get_addresses().iter())?;
    let txt = |k: &str| {
        info.get_property_val_str(k)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    Some(DiscoveredPal {
        id: txt("id")
            .map(|s| normalize_id(&s))
            .unwrap_or_else(|| "unknown".to_string()),
        name: txt("name").unwrap_or_else(|| "Cubi".to_string()),
        ip,
        port: info.get_port(),
        model: txt("model").unwrap_or_else(|| "unknown".to_string()),
        chip: txt("chip").map(|c| c.to_ascii_lowercase()),
        // `ver` per the round-2 contract; `fw` from earlier firmware.
        fw: crate::ota::version_label(txt("ver").or_else(|| txt("fw")).as_deref()),
    })
}

/// Chooses the address the app will use in `http://{addr}/...` URLs: an IPv4
/// address when there is one (lowest, for a stable choice), else a routable
/// IPv6 address in brackets. Link-local IPv6 (`fe80::/10`) would need a scope
/// id, so it is never used.
fn pick_pal_address<'a>(addrs: impl Iterator<Item = &'a IpAddr>) -> Option<String> {
    let mut v4 = None;
    let mut v6 = None;
    for addr in addrs {
        match addr {
            IpAddr::V4(a) if !a.is_unspecified() => {
                v4 = Some(v4.map_or(*a, |b: std::net::Ipv4Addr| b.min(*a)));
            }
            IpAddr::V6(a) if !a.is_unspecified() && (a.segments()[0] & 0xffc0) != 0xfe80 => {
                v6 = Some(v6.map_or(*a, |b: std::net::Ipv6Addr| b.min(*a)));
            }
            _ => {}
        }
    }
    v4.map(|a| a.to_string())
        .or_else(|| v6.map(|a| format!("[{}]", a)))
}

// ---------------------------------------------------------------------------
// Identity checks (contract §5)
// ---------------------------------------------------------------------------

/// What `GET /api/info` (unauthenticated) says about the Pal at an address.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceInfo {
    pub id: Option<String>,
    pub chip: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub fw: Option<String>,
}

impl DeviceInfo {
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref().or(self.fw.as_deref())
    }
}

#[derive(Debug)]
enum Probe {
    /// The Pal answered `/api/info` with an id.
    Identified(DeviceInfo),
    /// `/api/info` is 404: firmware before 1.1.0, which doesn't have it.
    Legacy,
    /// Something answered HTTP but gave no usable id.
    Anonymous,
    /// Nothing answered.
    Unreachable,
}

fn short_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(2500))
        .build()
        .map_err(|e| e.to_string())
}

/// Asks the host at `ip` who it is. Never sends the token.
async fn probe_identity(client: &reqwest::Client, ip: &str) -> Probe {
    let res = match client.get(format!("http://{}/api/info", ip)).send().await {
        Ok(r) => r,
        Err(_) => return Probe::Unreachable,
    };
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Probe::Legacy;
    }
    if !res.status().is_success() {
        return Probe::Anonymous;
    }
    match res.json::<DeviceInfo>().await {
        Ok(info)
            if info
                .id
                .as_deref()
                .map(normalize_id)
                .is_some_and(|i| is_valid_id(&i)) =>
        {
            Probe::Identified(info)
        }
        _ => Probe::Anonymous,
    }
}

/// `/api/info` for the paired Pal (no token needed); `None` on older firmware.
pub async fn fetch_device_info(ip: &str) -> Option<DeviceInfo> {
    let client = short_client().ok()?;
    match probe_identity(&client, ip).await {
        Probe::Identified(info) => Some(info),
        _ => None,
    }
}

/// A Pal the token may be sent to.
#[derive(Debug, Clone)]
pub struct Target {
    pub ip: String,
    pub token: String,
    pub id: Option<String>,
    /// Firmware before 1.1.0, trusted at its stored address without an id
    /// check (B-4).
    pub legacy: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveError {
    NotPaired,
    /// The paired Pal isn't answering at its address and wasn't found on the
    /// network.
    Offline {
        ip: String,
    },
    /// A different device answers at the paired address and the paired Pal
    /// wasn't found elsewhere.
    Mismatch {
        ip: String,
    },
}

impl ResolveError {
    pub fn message(&self) -> String {
        match self {
            ResolveError::NotPaired => "No CubiPal is currently paired.".to_string(),
            ResolveError::Offline { .. } => UNREACHABLE_MSG.to_string(),
            ResolveError::Mismatch { ip } => format!(
                "A different device now answers at {}, and your paired CubiPal wasn't found on this network. Nothing was sent to it.",
                ip
            ),
        }
    }
}

struct VerifyCache {
    /// (ip, id) last verified, when, and whether it was a legacy pairing
    /// accepted without an id check. The id is "" for a pairing with none.
    verified: Option<(String, String, Instant, bool)>,
    /// When the last failed search for the paired Pal ended.
    last_failed_search: Option<(String, Instant, ResolveError)>,
}

static VERIFY_CACHE: StdMutex<VerifyCache> = StdMutex::new(VerifyCache {
    verified: None,
    last_failed_search: None,
});
/// Serialises identity checks so parallel callers share one mDNS search.
static RESOLVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn forget_verification() {
    if let Ok(mut c) = VERIFY_CACHE.lock() {
        c.verified = None;
        c.last_failed_search = None;
    }
}

/// Call after a request to the Pal failed at the network level, so the next
/// call checks its identity (and address) again.
pub fn note_unreachable() {
    if let Ok(mut c) = VERIFY_CACHE.lock() {
        c.verified = None;
    }
}

/// The saved pairing as a target, if it was checked within the last minute.
fn cached_target(saved: &SavedDevice) -> Option<Target> {
    let id = saved.id.as_deref().unwrap_or_default();
    let c = VERIFY_CACHE.lock().ok()?;
    let (vip, vid, at, legacy) = c.verified.as_ref()?;
    (*vip == saved.ip && vid == id && at.elapsed() < VERIFIED_TTL).then(|| Target {
        ip: saved.ip.clone(),
        token: saved.token.clone(),
        id: saved.id.clone(),
        legacy: *legacy,
    })
}

fn remember_verified(ip: &str, id: &str, legacy: bool) {
    if let Ok(mut c) = VERIFY_CACHE.lock() {
        c.verified = Some((ip.to_string(), id.to_string(), Instant::now(), legacy));
        c.last_failed_search = None;
    }
}

fn recent_failed_search(id: &str) -> Option<ResolveError> {
    let c = VERIFY_CACHE.lock().ok()?;
    let (fid, at, err) = c.last_failed_search.as_ref()?;
    (fid == id && at.elapsed() < RELOCATE_RETRY).then(|| err.clone())
}

fn remember_failed_search(id: &str, err: &ResolveError) {
    if let Ok(mut c) = VERIFY_CACHE.lock() {
        c.last_failed_search = Some((id.to_string(), Instant::now(), err.clone()));
    }
}

/// Upgrades a stored legacy 4-digit id to the full id, and records an id for
/// pairings made before ids were stored.
fn capture_id(saved: &SavedDevice, seen: &str) {
    let seen = normalize_id(seen);
    if !is_valid_id(&seen) {
        return;
    }
    let new = match saved.id.as_deref() {
        None => seen,
        Some(stored) if ids_match(stored, &seen) => preferred_id(stored, &seen),
        Some(_) => return,
    };
    if saved.id.as_deref() != Some(new.as_str()) {
        let _ = keyring_set(KEYRING_ID_USER, &new);
    }
}

/// What the probe of the stored address means for the saved pairing.
#[derive(Debug, PartialEq)]
enum AtStored {
    /// The paired Pal answers there; this is its (best) id.
    Verified(String),
    /// Firmware before 1.1.0 (no `/api/info`) and a pairing without a full
    /// id: use the stored address, as 0.1.x did, and suggest an update.
    Legacy,
    /// Look the Pal up by id over mDNS; `mismatch` when a different device
    /// answers at the stored address.
    Search { mismatch: bool },
    /// No id to search with, and nothing answers.
    Offline,
}

/// Decides how to treat the stored address (B-4, B-5). A pairing with a full
/// id is always checked strictly; only a pairing with no id or a legacy
/// 4-digit id may fall back to trusting the address of a pre-1.1.0 Pal.
fn judge_stored(stored_id: Option<&str>, probe: &Probe) -> AtStored {
    match (stored_id, probe) {
        (_, Probe::Identified(info)) => {
            let seen = normalize_id(info.id.as_deref().unwrap_or_default());
            match stored_id {
                // Nothing to compare with: record the id before any token
                // is sent, and check it strictly from then on.
                None => AtStored::Verified(seen),
                Some(stored) if ids_match(stored, &seen) => {
                    AtStored::Verified(preferred_id(stored, &seen))
                }
                Some(_) => AtStored::Search { mismatch: true },
            }
        }
        (None, Probe::Legacy | Probe::Anonymous) => AtStored::Legacy,
        (None, Probe::Unreachable) => AtStored::Offline,
        (Some(stored), Probe::Legacy) if is_legacy_id(stored) => AtStored::Legacy,
        (Some(_), _) => AtStored::Search { mismatch: false },
    }
}

/// Returns the paired Pal's current address, checked to be the paired Pal,
/// so the token can be sent to it.
///
/// - The id at the stored address is checked with `GET /api/info` first
///   (never with the token).
/// - On no answer, or a different id, the Pal is looked up by id over mDNS
///   and, if found, its new address is checked and saved.
/// - The token is never sent to an address that reports a different id.
/// - Pairings saved before 0.2.0 have no id: the id `/api/info` reports is
///   recorded first. Firmware too old to have `/api/info` is used at the
///   stored address, for a pairing with no id or a legacy 4-digit id only.
pub async fn resolve_paired_device() -> Result<Target, ResolveError> {
    let saved = load_saved_device().ok_or(ResolveError::NotPaired)?;
    if let Some(target) = cached_target(&saved) {
        return Ok(target);
    }

    let _guard = RESOLVE_LOCK.lock().await;
    // Another caller may have finished the check while we waited.
    let saved = load_saved_device().ok_or(ResolveError::NotPaired)?;
    if let Some(target) = cached_target(&saved) {
        return Ok(target);
    }

    let client = short_client().map_err(|_| ResolveError::Offline {
        ip: saved.ip.clone(),
    })?;

    let at_stored = probe_identity(&client, &saved.ip).await;
    let mismatch_at_stored = match judge_stored(saved.id.as_deref(), &at_stored) {
        AtStored::Verified(id) => {
            capture_id(&saved, &id);
            remember_verified(&saved.ip, &id, false);
            return Ok(Target {
                ip: saved.ip,
                token: saved.token,
                id: Some(id),
                legacy: false,
            });
        }
        AtStored::Legacy => {
            remember_verified(&saved.ip, saved.id.as_deref().unwrap_or_default(), true);
            return Ok(Target {
                ip: saved.ip,
                token: saved.token,
                id: saved.id,
                legacy: true,
            });
        }
        AtStored::Offline => return Err(ResolveError::Offline { ip: saved.ip }),
        AtStored::Search { mismatch } => mismatch,
    };
    // Only a pairing with an id is ever searched for.
    let Some(stored_id) = saved.id.clone() else {
        return Err(ResolveError::Offline { ip: saved.ip });
    };

    if let Some(err) = recent_failed_search(&stored_id) {
        return Err(err);
    }

    // Not verified at the stored address: look the Pal up by id.
    let found = mdns_find_by_id(&stored_id).await;
    let result = match found {
        Some(pal) => verify_found(&client, &saved, &stored_id, pal).await,
        None if mismatch_at_stored => Err(ResolveError::Mismatch {
            ip: saved.ip.clone(),
        }),
        None => Err(ResolveError::Offline {
            ip: saved.ip.clone(),
        }),
    };
    if let Err(ref e) = result {
        remember_failed_search(&stored_id, e);
    }
    result
}

/// Checks a Pal that mDNS says has our id, and saves its address.
async fn verify_found(
    client: &reqwest::Client,
    saved: &SavedDevice,
    stored_id: &str,
    pal: DiscoveredPal,
) -> Result<Target, ResolveError> {
    let (id, legacy) = match probe_identity(client, &pal.ip).await {
        Probe::Identified(info) => {
            let seen = info.id.as_deref().unwrap_or_default();
            if !ids_match(stored_id, seen) {
                return Err(ResolveError::Mismatch {
                    ip: saved.ip.clone(),
                });
            }
            (preferred_id(stored_id, seen), false)
        }
        // Firmware before 1.1.0 can only be identified by its mDNS id.
        Probe::Legacy | Probe::Anonymous => (preferred_id(stored_id, &pal.id), true),
        Probe::Unreachable => {
            return Err(ResolveError::Offline {
                ip: saved.ip.clone(),
            });
        }
    };
    if pal.ip != saved.ip {
        keyring_set(KEYRING_IP_USER, &pal.ip).map_err(|_| ResolveError::Offline {
            ip: saved.ip.clone(),
        })?;
    }
    capture_id(saved, &id);
    remember_verified(&pal.ip, &id, legacy);
    Ok(Target {
        ip: pal.ip,
        token: saved.token.clone(),
        id: Some(id),
        legacy,
    })
}

/// [`resolve_paired_device`] with a user-facing error.
pub async fn paired_target() -> Result<Target, String> {
    resolve_paired_device().await.map_err(|e| e.message())
}

/// Text for a failed request to the Pal; also forgets the identity check so
/// the next call re-checks the address.
pub fn reach_error(e: reqwest::Error) -> String {
    note_unreachable();
    eprintln!("CubiPal request failed: {}", e.without_url());
    UNREACHABLE_MSG.to_string()
}

/// The Pal's own explanation from a failed API call (`{"ok":false,"msg":...}`),
/// else a short excerpt of the body, else a generic line. 401 and 409 get
/// plain-language text; the HTTP status is logged, not shown.
pub async fn device_error(res: reqwest::Response) -> String {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    describe_device_error(status.as_u16(), &body)
}

pub fn describe_device_error(status: u16, body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["msg"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    let msg = msg.trim();
    eprintln!("CubiPal answered HTTP {}: {}", status, msg);
    match status {
        401 => "Your CubiPal no longer accepts this app's pairing (it may have been reset). Unpair and pair it again.".to_string(),
        409 => BUSY_MSG.to_string(),
        _ if msg.is_empty() => {
            "Your Pal couldn't do that right now. Try again in a moment.".to_string()
        }
        _ => msg.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Pairs with the Pal at `ip`. `id` is the mDNS id from the scan, if the Pal
/// was found that way; the id the Pal itself reports wins (trust on first use).
#[tauri::command]
pub async fn pair_pal(
    ip: String,
    code: String,
    id: Option<String>,
) -> Result<PairResponse, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let clean_ip = ip
        .trim()
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
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
        .map_err(|e| {
            eprintln!("Pairing request failed: {}", e.without_url());
            format!(
                "Couldn't reach a Pal at {} \u{2014} is it on and on the same Wi-Fi?",
                clean_ip
            )
        })?;

    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let parsed: PairResponse = serde_json::from_str(&body).map_err(|_| {
        format!(
            "Invalid response from device: {}",
            body.chars().take(200).collect::<String>()
        )
    })?;

    if !(status.is_success() && parsed.ok) {
        return Err(parsed
            .msg
            .unwrap_or_else(|| "The Pal didn't accept the pairing. Try again.".to_string()));
    }
    let Some(ref token) = parsed.token else {
        return Err("The Pal accepted the code but sent no token.".to_string());
    };

    // Trust on first use: the id the Pal reports (pair response, then
    // /api/info), else the id it advertised over mDNS.
    let mut device_id = parsed
        .id
        .as_deref()
        .map(normalize_id)
        .filter(|i| is_valid_id(i));
    if device_id.is_none()
        && let Some(info) = fetch_device_info(&clean_ip).await
    {
        device_id = info.id.as_deref().map(normalize_id);
    }
    if device_id.is_none() {
        device_id = id.as_deref().map(normalize_id).filter(|i| is_valid_id(i));
    }
    store_device(&clean_ip, token, device_id.as_deref())?;
    // A full id was just confirmed by the Pal itself. (A legacy 4-digit id
    // is checked on first use instead.)
    if let Some(ref id) = device_id
        && !is_legacy_id(id)
    {
        remember_verified(&clean_ip, id, false);
    }
    Ok(parsed)
}

#[tauri::command]
pub async fn get_paired_device() -> Result<Option<DeviceState>, String> {
    let target = match resolve_paired_device().await {
        Ok(t) => t,
        Err(ResolveError::NotPaired) => return Ok(None),
        Err(e) => {
            let saved = load_saved_device();
            let ip = match &e {
                ResolveError::Offline { ip } | ResolveError::Mismatch { ip } => ip.clone(),
                ResolveError::NotPaired => String::new(),
            };
            let problem = matches!(e, ResolveError::Mismatch { .. }).then(|| e.message());
            return Ok(Some(DeviceState::unreachable(
                ip,
                saved.and_then(|s| s.id),
                problem,
            )));
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;

    let res = match client
        .get(format!("http://{}/api/state", target.ip))
        .header("X-Pet", "1")
        .bearer_auth(&target.token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            note_unreachable();
            return Ok(Some(DeviceState::unreachable(target.ip, target.id, None)));
        }
    };

    if !res.status().is_success() {
        let problem = (res.status().as_u16() == 401).then(|| describe_device_error(401, ""));
        return Ok(Some(DeviceState::unreachable(
            target.ip, target.id, problem,
        )));
    }

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let str_field = |k: &str| {
        val[k]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };

    // Migration: capture the id of a Pal paired before ids were stored, or
    // upgrade a legacy 4-digit id.
    let reported_id = str_field("id").map(|s| normalize_id(&s));
    if let (Some(seen), Some(saved)) = (reported_id.as_deref(), load_saved_device())
        && saved.ip == target.ip
        && saved.token == target.token
    {
        capture_id(&saved, seen);
    }

    let mut fw = str_field("version").or_else(|| str_field("fw"));
    let mut chip = str_field("chip").map(|c| c.to_ascii_lowercase());
    let mut model = str_field("model");
    // Board type: /api/state (1.1.0+), else /api/info, else the mDNS TXT
    // record (firmware that advertises `chip` but has no /api/info).
    if chip.is_none()
        && let Some(info) = fetch_device_info(&target.ip).await
    {
        chip = info.chip.as_deref().map(str::to_ascii_lowercase);
        model = model.or(info.model.clone());
        fw = fw.or(info.version().map(String::from));
    }
    if chip.is_none()
        && let Some(pal) = cached_mdns_details(&target.ip).await
    {
        chip = pal.chip;
        model = model.or(Some(pal.model));
    }

    Ok(Some(DeviceState {
        robot: str_field("robot").unwrap_or_else(|| "Cubi".to_string()),
        owner: str_field("owner").unwrap_or_default(),
        mood: str_field("mood"),
        volume: val["volume"].as_u64().map(|v| v.min(255) as u8),
        muted: val["muted"].as_bool(),
        ssid: str_field("ssid"),
        rssi: val["rssi"].as_i64().map(|r| r as i32),
        fw_label: crate::ota::version_label(fw.as_deref()),
        fw,
        chip,
        model,
        id: reported_id.or(target.id),
        skin: str_field("skin"),
        ip: target.ip,
        connected: true,
        problem: None,
        legacy_pairing: target.legacy,
    }))
}

/// mDNS details (chip, model) for Pals whose `/api/state` doesn't include
/// them, looked up once per address and remembered.
async fn cached_mdns_details(ip: &str) -> Option<DiscoveredPal> {
    static CACHE: StdMutex<Option<(String, Instant, Option<DiscoveredPal>)>> = StdMutex::new(None);
    const TTL: Duration = Duration::from_secs(30 * 60);
    if let Ok(c) = CACHE.lock()
        && let Some((cip, at, pal)) = c.as_ref()
        && cip == ip
        && at.elapsed() < TTL
    {
        return pal.clone();
    }
    let pal = mdns_find_by_ip(ip).await;
    if let Ok(mut c) = CACHE.lock() {
        *c = Some((ip.to_string(), Instant::now(), pal.clone()));
    }
    pal
}

#[tauri::command]
pub async fn unpair_pal() -> Result<(), String> {
    clear_device_credentials()
}

#[tauri::command]
pub async fn set_device_skin(skin: String) -> Result<(), String> {
    let target = paired_target().await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .post(format!("http://{}/api/skin", target.ip))
        .header("X-Pet", "1")
        .bearer_auth(&target.token)
        .form(&[("id", skin.as_str())])
        .send()
        .await
        .map_err(reach_error)?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(device_error(res).await)
    }
}

#[tauri::command]
pub async fn start_device_timer(minutes: u32, label: String) -> Result<(), String> {
    // The Pal accepts 1-180 minutes; 0 stops the timer.
    if minutes > 180 {
        return Err("Focus timers can be at most 180 minutes.".to_string());
    }
    let target = paired_target().await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let form = [("min", minutes.to_string()), ("label", label)];

    let res = client
        .post(format!("http://{}/api/timer", target.ip))
        .bearer_auth(&target.token)
        .header("X-Pet", "1")
        .form(&form)
        .send()
        .await
        .map_err(reach_error)?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(device_error(res).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(addrs: &[&str]) -> Option<String> {
        let parsed: Vec<IpAddr> = addrs.iter().map(|a| a.parse().unwrap()).collect();
        pick_pal_address(parsed.iter())
    }

    #[test]
    fn prefers_ipv4() {
        assert_eq!(
            pick(&["fe80::1", "192.168.1.20", "2001:db8::5"]),
            Some("192.168.1.20".into())
        );
        assert_eq!(
            pick(&["192.168.1.30", "192.168.1.20"]),
            Some("192.168.1.20".into())
        );
    }

    #[test]
    fn falls_back_to_bracketed_routable_ipv6() {
        assert_eq!(
            pick(&["fe80::1", "2001:db8::5"]),
            Some("[2001:db8::5]".into())
        );
        assert_eq!(pick(&["fe80::1"]), None);
        assert_eq!(pick(&[]), None);
        assert_eq!(pick(&["0.0.0.0"]), None);
    }

    #[test]
    fn ids_normalise_and_match() {
        assert_eq!(normalize_id(" A0:B1:C2:D3:E4:F5 "), "a0b1c2d3e4f5");
        assert!(ids_match("a0b1c2d3e4f5", "A0-B1-C2-D3-E4-F5"));
        assert!(!ids_match("a0b1c2d3e4f5", "a0b1c2d3e4f6"));
        // A legacy 4-digit mDNS id matches the full id that ends with it.
        assert!(ids_match("e4f5", "a0b1c2d3e4f5"));
        assert!(ids_match("a0b1c2d3e4f5", "e4f5"));
        assert!(!ids_match("e4f6", "a0b1c2d3e4f5"));
        // 4 digits never match 4 different digits or a non-suffix.
        assert!(!ids_match("a0b1", "a0b1c2d3e4f5"));
        // Garbage never matches, even itself.
        assert!(!ids_match("unknown", "unknown"));
        assert!(!ids_match("", ""));
        assert_eq!(preferred_id("e4f5", "A0B1C2D3E4F5"), "a0b1c2d3e4f5");
        assert_eq!(preferred_id("a0b1c2d3e4f5", "e4f5"), "a0b1c2d3e4f5");
    }

    #[test]
    fn device_errors_show_the_pals_reason() {
        // 409: a timer or bedtime is running, whatever the Pal's wording.
        for body in [
            r#"{"ok":false,"msg":"Update rejected: a focus or countdown timer is active on CubiPal"}"#,
            r#"{"ok":false,"msg":"In bedtime right now"}"#,
        ] {
            assert_eq!(describe_device_error(409, body), BUSY_MSG);
        }
        // Otherwise the Pal's own reason, without the HTTP code.
        assert_eq!(
            describe_device_error(400, r#"{"ok":false,"msg":"Unknown skin"}"#),
            "Unknown skin"
        );
        assert_eq!(describe_device_error(500, "Update failed"), "Update failed");
        assert!(!describe_device_error(502, "").contains("HTTP"));
        assert!(describe_device_error(401, "{}").contains("pair it again"));
        assert!(ResolveError::Offline { ip: "x".into() }.message() == UNREACHABLE_MSG);
    }

    #[test]
    fn device_info_reads_the_firmware_contract() {
        let info: DeviceInfo = serde_json::from_str(
            r#"{"id":"a0b1c2d3e4f5","chip":"esp32s3","model":"v7","version":"1.1.0","fw":"1.1.0","name":"Cubi","proto":1}"#,
        )
        .unwrap();
        assert_eq!(info.id.as_deref(), Some("a0b1c2d3e4f5"));
        assert_eq!(info.chip.as_deref(), Some("esp32s3"));
        assert_eq!(info.version(), Some("1.1.0"));
        let legacy: DeviceInfo = serde_json::from_str(r#"{"fw":"1 Nov 12 2026"}"#).unwrap();
        assert_eq!(legacy.version(), Some("1 Nov 12 2026"));
    }

    fn identified(id: &str) -> Probe {
        Probe::Identified(DeviceInfo {
            id: Some(id.to_string()),
            ..Default::default()
        })
    }

    #[test]
    fn legacy_firmware_is_trusted_only_for_pairings_without_a_full_id() {
        const FULL: &str = "a0b1c2d3e4f5";
        use AtStored::*;
        // No stored id (paired before 0.2.0).
        assert_eq!(judge_stored(None, &Probe::Legacy), Legacy);
        assert_eq!(judge_stored(None, &Probe::Anonymous), Legacy);
        assert_eq!(judge_stored(None, &Probe::Unreachable), Offline);
        // B-5: the id is learned from /api/info before the token is sent.
        assert_eq!(
            judge_stored(None, &identified("A0:B1:C2:D3:E4:F5")),
            Verified(FULL.into())
        );
        // Legacy 4-digit id (scanned from pre-1.1.0 mDNS).
        assert_eq!(judge_stored(Some("e4f5"), &Probe::Legacy), Legacy);
        assert_eq!(
            judge_stored(Some("e4f5"), &identified(FULL)),
            Verified(FULL.into())
        );
        assert_eq!(
            judge_stored(Some("e4f5"), &identified("0123456789ab")),
            Search { mismatch: true }
        );
        assert_eq!(
            judge_stored(Some("e4f5"), &Probe::Anonymous),
            Search { mismatch: false }
        );
        assert_eq!(
            judge_stored(Some("e4f5"), &Probe::Unreachable),
            Search { mismatch: false }
        );
        // A full id stays strict: a 404 at the stored address is never trusted.
        assert_eq!(
            judge_stored(Some(FULL), &Probe::Legacy),
            Search { mismatch: false }
        );
        assert_eq!(
            judge_stored(Some(FULL), &Probe::Anonymous),
            Search { mismatch: false }
        );
        assert_eq!(
            judge_stored(Some(FULL), &identified("0123456789ab")),
            Search { mismatch: true }
        );
        assert_eq!(
            judge_stored(Some(FULL), &identified(FULL)),
            Verified(FULL.into())
        );
    }

    async fn serve_once(status: &'static str, body: &'static str) -> String {
        crate::test_http::serve(vec![(status, body)]).await.0
    }

    #[tokio::test]
    async fn probe_tells_legacy_firmware_from_other_answers() {
        let client = short_client().unwrap();
        let ip = serve_once("404 Not Found", r#"{"ok":false}"#).await;
        assert!(matches!(probe_identity(&client, &ip).await, Probe::Legacy));
        let ip = serve_once("500 Internal Server Error", "").await;
        assert!(matches!(
            probe_identity(&client, &ip).await,
            Probe::Anonymous
        ));
        let ip = serve_once("200 OK", r#"{"chip":"esp32"}"#).await;
        assert!(matches!(
            probe_identity(&client, &ip).await,
            Probe::Anonymous
        ));
        let ip = serve_once("200 OK", r#"{"id":"a0b1c2d3e4f5","chip":"esp32s3"}"#).await;
        assert!(matches!(
            probe_identity(&client, &ip).await,
            Probe::Identified(_)
        ));
        // Nothing listening.
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ip = free.local_addr().unwrap().to_string();
        drop(free);
        assert!(matches!(
            probe_identity(&client, &ip).await,
            Probe::Unreachable
        ));
    }
}
