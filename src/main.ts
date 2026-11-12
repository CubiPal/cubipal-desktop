import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

interface DeviceState {
  robot: string;
  owner: string;
  mood?: string;
  volume?: number;
  muted?: boolean;
  ssid?: string;
  rssi?: number;
  fw?: string;
  fw_label: string;
  chip?: string;
  model?: string;
  id?: string;
  skin?: string;
  ip: string;
  connected: boolean;
  problem?: string;
}

interface DiscoveredPal {
  id: string;
  name: string;
  ip: string;
  port: number;
  model: string;
  chip?: string;
  fw: string;
}

interface PairResponse {
  ok: boolean;
  token?: string;
  robot?: string;
  owner?: string;
  fw?: string;
  msg?: string;
}

interface ConnectedAccounts {
  google_connected: boolean;
  microsoft_connected: boolean;
  microsoft_available: boolean;
  ics_url: string | null;
}

interface CalendarSyncSettings {
  private_mode: boolean;
  skip_all_day: boolean;
  skip_tentative: boolean;
}

interface CalendarSyncConfig {
  private_mode: boolean;
  skip_all_day: boolean;
  skip_tentative: boolean;
  leadTimeMin: number;
}

interface CalendarEvent {
  id: string;
  title: string;
  start: number; // Unix epoch seconds
  end: number;
  source: string;
  is_all_day: boolean;
  is_tentative: boolean;
}

interface FirmwareUpdateInfo {
  available: boolean;
  latest_version: string | null;
  current_version: string;
  chip: string | null;
  blocked_reason: string | null;
  notes_url: string | null;
}

interface DesktopUpdateInfo {
  version: string;
  current_version: string;
  body?: string;
}

interface GeneralSettings {
  autostart: boolean;
  auto_update: boolean;
  app_version: string;
}

interface OtaProgress {
  stage: string;
  percent: number;
  message: string;
}

let currentDeviceIp: string | null = null;
let firmwareUpdateReady = false;
// Only re-check for firmware when the Pal's version changes, or hourly (the
// backend also caches cubipal.com/firmware/latest.json for an hour).
const FIRMWARE_CHECK_INTERVAL_MS = 60 * 60 * 1000;
let lastFirmwareCheckAt = 0;
let lastFirmwareCheckVersion: string | undefined;

document.addEventListener("DOMContentLoaded", async () => {
  setupTabs();
  setupPairingModal();
  setupActionButtons();
  setupCalendarTab();
  setupRemindersTab();
  setupTrayListeners();
  setupOta();
  setupGeneralTab();

  // Load initial paired device status and calendars
  await refreshDeviceStatus();
  await refreshConnectedAccounts();
  await loadCalendarSyncConfig();
  await syncAndPreviewEvents();

  // Poll state every 15 seconds
  setInterval(refreshDeviceStatus, 15000);

  // Background calendar sync and event push every 5 minutes (KAN-33)
  setInterval(syncAndPreviewEvents, 300000);
});

function setupTabs() {
  const navItems = document.querySelectorAll<HTMLButtonElement>(".nav-item");
  const tabPanes = document.querySelectorAll<HTMLElement>(".tab-pane");

  navItems.forEach((btn) => {
    btn.addEventListener("click", async () => {
      const targetTab = btn.getAttribute("data-tab");
      if (!targetTab) return;

      navItems.forEach((item) => item.classList.remove("active"));
      tabPanes.forEach((pane) => pane.classList.remove("active"));

      btn.classList.add("active");
      const activePane = document.getElementById(`tab-${targetTab}`);
      if (activePane) {
        activePane.classList.add("active");
      }

      if (targetTab === "calendars") {
        await refreshConnectedAccounts();
        await syncAndPreviewEvents();
      }
    });
  });
}

async function refreshDeviceStatus() {
  try {
    const dev = await invoke<DeviceState | null>("get_paired_device");

    const nameEl = document.getElementById("device-name")!;
    const subtitleEl = document.getElementById("device-subtitle")!;
    const ipEl = document.getElementById("spec-ip")!;
    const wifiEl = document.getElementById("spec-wifi")!;
    const fwEl = document.getElementById("spec-fw")!;
    const volumeEl = document.getElementById("spec-volume")!;
    const skinEl = document.getElementById("spec-skin") as HTMLSelectElement | null;
    const pill = document.getElementById("sidebar-status-pill")!;
    const pillText = document.getElementById("sidebar-status-text")!;
    const btnOpenWeb = document.getElementById("btn-open-web") as HTMLButtonElement;
    const btnUnpair = document.getElementById("btn-unpair") as HTMLButtonElement;
    const btnTestChime = document.getElementById("btn-test-chime") as HTMLButtonElement;

    if (dev && dev.connected) {
      currentDeviceIp = dev.ip;
      nameEl.textContent = dev.owner ? `${dev.robot} (${dev.owner}'s Pal)` : dev.robot;
      subtitleEl.textContent = "Paired and connected over local Wi-Fi";
      ipEl.textContent = `http://${dev.ip}`;
      wifiEl.textContent = dev.ssid ? `${dev.ssid} (${dev.rssi || 0} dBm)` : "Connected";
      fwEl.textContent = dev.fw_label;
      volumeEl.textContent = dev.muted ? "Muted" : `Level ${dev.volume ?? 3}`;
      if (skinEl) {
        skinEl.disabled = false;
        if (document.activeElement !== skinEl && dev.skin) {
          skinEl.value = dev.skin;
        }
      }

      pill.className = "status-pill online";
      pillText.textContent = "Online";

      btnOpenWeb.disabled = false;
      btnUnpair.disabled = false;
      if (btnTestChime) btnTestChime.disabled = false;
      if (
        dev.fw !== lastFirmwareCheckVersion ||
        Date.now() - lastFirmwareCheckAt > FIRMWARE_CHECK_INTERVAL_MS
      ) {
        lastFirmwareCheckAt = Date.now();
        lastFirmwareCheckVersion = dev.fw;
        await checkFirmwareUpdate();
      }
    } else if (dev && !dev.connected) {
      document.getElementById("ota-card")?.classList.add("hidden");
      lastFirmwareCheckAt = 0; // re-check as soon as the Pal is back
      currentDeviceIp = dev.ip;
      nameEl.textContent = `${dev.robot} (Offline)`;
      subtitleEl.textContent = dev.problem || "Cannot reach device on the network";
      ipEl.textContent = dev.ip ? `http://${dev.ip} (Unreachable)` : "Unreachable";
      wifiEl.textContent = "Offline";
      fwEl.textContent = "—";
      volumeEl.textContent = "—";
      if (skinEl) skinEl.disabled = true;

      pill.className = "status-pill offline";
      pillText.textContent = "Offline";

      btnOpenWeb.disabled = true;
      btnUnpair.disabled = false;
      if (btnTestChime) btnTestChime.disabled = true;
    } else {
      document.getElementById("ota-card")?.classList.add("hidden");
      lastFirmwareCheckAt = 0; // re-check as soon as the Pal is back
      currentDeviceIp = null;
      nameEl.textContent = "No CubiPal Paired";
      subtitleEl.textContent = "Click Scan Network to find your Pal on this Wi-Fi, then use the show-code gesture to get its pairing code.";
      ipEl.textContent = "Not connected";
      wifiEl.textContent = "—";
      fwEl.textContent = "—";
      volumeEl.textContent = "—";
      if (skinEl) skinEl.disabled = true;

      pill.className = "status-pill offline";
      pillText.textContent = "Disconnected";

      btnOpenWeb.disabled = true;
      btnUnpair.disabled = true;
      if (btnTestChime) btnTestChime.disabled = true;
    }
  } catch (err) {
    console.error("Failed to query paired device:", err);
  }
}

function setupActionButtons() {
  const btnScan = document.getElementById("btn-scan")!;
  const scanCard = document.getElementById("scan-results-card")!;
  const scanList = document.getElementById("scan-list")!;
  const scanTitle = document.getElementById("scan-title")!;

  btnScan.addEventListener("click", async () => {
    scanCard.classList.remove("hidden");
    scanTitle.textContent = "Scanning local Wi-Fi for CubiPals...";
    scanList.innerHTML = `<p class="text-muted text-sm py-2">Listening for mDNS advertisements (_cubipal._tcp)...</p>`;

    try {
      const pals = await invoke<DiscoveredPal[]>("scan_for_pals");
      if (pals.length === 0) {
        scanTitle.textContent = "No CubiPals Found";
        scanList.innerHTML = `
          <div class="text-muted text-sm py-2">
            No devices replied to mDNS discovery. Make sure your Pal is plugged in and on the same Wi-Fi.
          </div>
        `;
      } else {
        scanTitle.textContent = `Discovered CubiPals (${pals.length})`;
        scanList.innerHTML = "";
        // mDNS names and TXT values come from any device on the LAN: only ever
        // insert them as text, never as HTML.
        pals.forEach((pal) => {
          const row = document.createElement("div");
          row.className = "discovered-row";

          const info = document.createElement("div");
          info.className = "discovered-info";
          const nameEl = document.createElement("strong");
          nameEl.textContent = pal.name;
          const meta = document.createElement("div");
          meta.className = "discovered-meta";
          meta.textContent = `${pal.ip} \u2022 Model ${pal.model} \u2022 Firmware ${pal.fw}`;
          info.append(nameEl, meta);

          const btn = document.createElement("button");
          btn.className = "btn btn-primary btn-pair-pal";
          btn.textContent = "Pair";
          btn.setAttribute("aria-label", `Pair with ${pal.name}`);
          btn.addEventListener("click", () => openPairingModal(pal.ip, pal.name, pal.id));

          row.append(info, btn);
          scanList.appendChild(row);
        });
      }
    } catch (e) {
      scanTitle.textContent = "Scan Error";
      const msg = document.createElement("p");
      msg.className = "text-muted text-sm";
      msg.textContent = String(e);
      scanList.replaceChildren(msg);
    }
  });

  const btnManualIp = document.getElementById("btn-manual-ip")!;
  btnManualIp.addEventListener("click", () => {
    const ip = prompt("Enter CubiPal IP address or hostname (e.g. 192.168.1.100 or cubipal-ab12.local):");
    if (ip && ip.trim()) {
      openPairingModal(ip.trim(), "CubiPal", null);
    }
  });

  const btnOpenWeb = document.getElementById("btn-open-web")!;
  btnOpenWeb.addEventListener("click", async () => {
    if (currentDeviceIp) {
      await openUrl(`http://${currentDeviceIp}`);
    }
  });

  const btnUnpair = document.getElementById("btn-unpair")!;
  btnUnpair.addEventListener("click", async () => {
    if (confirm("Are you sure you want to unpair this CubiPal?")) {
      await invoke("unpair_pal");
      await refreshDeviceStatus();
    }
  });

  const skinEl = document.getElementById("spec-skin") as HTMLSelectElement | null;
  skinEl?.addEventListener("change", async () => {
    if (!skinEl.value) return;
    try {
      await invoke("set_device_skin", { skin: skinEl.value });
    } catch (e: any) {
      console.error("Failed to update skin:", e);
    }
  });
}

function setupPairingModal() {
  const modal = document.getElementById("pairing-modal")!;
  const btnClose = document.getElementById("btn-modal-close")!;
  const btnCancel = document.getElementById("btn-modal-cancel")!;
  const btnPair = document.getElementById("btn-modal-pair") as HTMLButtonElement;
  const inputCode = document.getElementById("input-pairing-code") as HTMLInputElement;
  const targetIpInput = document.getElementById("modal-target-ip") as HTMLInputElement;
  const targetIdInput = document.getElementById("modal-target-id") as HTMLInputElement;
  const errorMsg = document.getElementById("pairing-error-msg")!;

  const closeModal = () => {
    modal.classList.add("hidden");
    inputCode.value = "";
    errorMsg.classList.add("hidden");
    btnPair.disabled = false;
    // Return focus to whatever opened the dialog (e.g. the Pair button).
    pairingModalOpener?.focus();
    pairingModalOpener = null;
  };

  btnClose.addEventListener("click", closeModal);
  btnCancel.addEventListener("click", closeModal);

  // Keyboard: Esc closes, Enter in the code field submits, Tab stays in the dialog.
  modal.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      closeModal();
    } else if (e.key === "Enter" && e.target === inputCode && !btnPair.disabled) {
      e.preventDefault();
      btnPair.click();
    } else if (e.key === "Tab") {
      const focusable = Array.from(
        modal.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([type=hidden])")
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  });

  // Auto-format "XXXX XXXX"
  inputCode.addEventListener("input", () => {
    let raw = inputCode.value.replace(/\D/g, "");
    if (raw.length > 4) {
      raw = raw.slice(0, 4) + " " + raw.slice(4, 8);
    }
    inputCode.value = raw;
  });

  btnPair.addEventListener("click", async () => {
    const ip = targetIpInput.value;
    const id = targetIdInput.value || null;
    const code = inputCode.value.trim();

    if (code.replace(/\s/g, "").length !== 8) {
      errorMsg.textContent = "Please enter all 8 digits shown on your CubiPal.";
      errorMsg.classList.remove("hidden");
      return;
    }

    btnPair.disabled = true;
    errorMsg.classList.add("hidden");

    try {
      const res = await invoke<PairResponse>("pair_pal", { ip, code, id });
      if (res.ok) {
        closeModal();
        const scanCard = document.getElementById("scan-results-card");
        scanCard?.classList.add("hidden");
        await refreshDeviceStatus();
        alert(`Successfully paired with ${res.robot || "CubiPal"}!`);
      } else {
        errorMsg.textContent = res.msg || "Pairing failed.";
        errorMsg.classList.remove("hidden");
        btnPair.disabled = false;
      }
    } catch (err) {
      errorMsg.textContent = String(err);
      errorMsg.classList.remove("hidden");
      btnPair.disabled = false;
    }
  });
}

let pairingModalOpener: HTMLElement | null = null;

function openPairingModal(ip: string, name: string, id: string | null) {
  pairingModalOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const modal = document.getElementById("pairing-modal")!;
  const title = document.getElementById("modal-pal-name")!;
  const targetIpInput = document.getElementById("modal-target-ip") as HTMLInputElement;
  const inputCode = document.getElementById("input-pairing-code") as HTMLInputElement;
  const errorMsg = document.getElementById("pairing-error-msg")!;

  title.textContent = `Pair with ${name}`;
  targetIpInput.value = ip;
  (document.getElementById("modal-target-id") as HTMLInputElement).value =
    id && id !== "unknown" ? id : "";
  inputCode.value = "";
  errorMsg.classList.add("hidden");

  modal.classList.remove("hidden");
  setTimeout(() => inputCode.focus(), 50);
}

// --------------------------------------------------------------------------
// Calendar Integrations (KAN-30 Google, KAN-31 Microsoft, KAN-32 ICS)
// --------------------------------------------------------------------------

function setupCalendarTab() {
  const btnGoogle = document.getElementById("btn-connect-google")!;
  const btnMs = document.getElementById("btn-connect-ms")!;
  const btnIcs = document.getElementById("btn-connect-ics")!;
  const btnRefresh = document.getElementById("btn-refresh-calendar")!;
  const togglePrivate = document.getElementById("toggle-private-mode")!;
  const toggleAllDay = document.getElementById("toggle-skip-allday")!;
  const toggleTentative = document.getElementById("toggle-skip-tentative")!;

  btnGoogle.addEventListener("click", async () => {
    try {
      const accs = await invoke<ConnectedAccounts>("get_connected_accounts");
      if (accs.google_connected) {
        if (confirm("Disconnect Google Calendar?")) {
          await invoke("disconnect_google_account");
          await refreshConnectedAccounts();
          await syncAndPreviewEvents();
        }
      } else {
        btnGoogle.textContent = "Authorizing...";
        btnGoogle.setAttribute("disabled", "true");
        try {
          await invoke("connect_google_account", { customClientId: null });
          await refreshConnectedAccounts();
          await syncAndPreviewEvents();
        } catch (err) {
          alert("Google authorization failed: " + err);
          await refreshConnectedAccounts();
        } finally {
          btnGoogle.removeAttribute("disabled");
        }
      }
    } catch (err) {
      console.error(err);
    }
  });

  btnMs.addEventListener("click", async () => {
    try {
      const accs = await invoke<ConnectedAccounts>("get_connected_accounts");
      if (accs.microsoft_connected) {
        if (confirm("Disconnect Microsoft 365 Calendar?")) {
          await invoke("disconnect_microsoft_account");
          await refreshConnectedAccounts();
          await syncAndPreviewEvents();
        }
      } else {
        btnMs.textContent = "Authorizing...";
        btnMs.setAttribute("disabled", "true");
        try {
          await invoke("connect_microsoft_account", { customClientId: null });
          await refreshConnectedAccounts();
          await syncAndPreviewEvents();
        } catch (err) {
          alert("Microsoft authorization failed: " + err);
          await refreshConnectedAccounts();
        } finally {
          btnMs.removeAttribute("disabled");
        }
      }
    } catch (err) {
      console.error(err);
    }
  });

  btnIcs.addEventListener("click", async () => {
    try {
      const accs = await invoke<ConnectedAccounts>("get_connected_accounts");
      if (accs.ics_url) {
        if (confirm(`Remove private calendar feed?\n\n${accs.ics_url}`)) {
          await invoke("disconnect_ics");
          await refreshConnectedAccounts();
          await syncAndPreviewEvents();
        }
      } else {
        const url = prompt(
          "Enter private calendar feed URL (.ics or webcal:// from Apple Calendar, Fastmail, Nextcloud, etc.):"
        );
        if (url && url.trim()) {
          try {
            await invoke("set_ics_url", { url: url.trim() });
            await refreshConnectedAccounts();
            await syncAndPreviewEvents();
          } catch (err) {
            alert("Failed to save ICS feed: " + err);
          }
        }
      }
    } catch (err) {
      console.error(err);
    }
  });

  btnRefresh.addEventListener("click", () => {
    syncAndPreviewEvents();
  });

  togglePrivate.addEventListener("change", () => syncAndPreviewEvents());
  toggleAllDay.addEventListener("change", () => syncAndPreviewEvents());
  toggleTentative.addEventListener("change", () => syncAndPreviewEvents());
}

async function refreshConnectedAccounts() {
  try {
    const accs = await invoke<ConnectedAccounts>("get_connected_accounts");

    const googleStatus = document.getElementById("google-status");
    const btnGoogle = document.getElementById("btn-connect-google") as HTMLButtonElement | null;
    if (googleStatus && btnGoogle) {
      if (accs.google_connected) {
        googleStatus.textContent = "Connected via OAuth (PKCE)";
        googleStatus.className = "text-success text-sm";
        btnGoogle.textContent = "Disconnect";
        btnGoogle.className = "btn btn-danger text-xs";
      } else {
        googleStatus.textContent = "Not connected";
        googleStatus.className = "text-muted text-sm";
        btnGoogle.textContent = "Connect";
        btnGoogle.className = "btn btn-secondary";
      }
    }

    const msStatus = document.getElementById("ms-status");
    const btnMs = document.getElementById("btn-connect-ms") as HTMLButtonElement | null;
    if (msStatus && btnMs) {
      btnMs.disabled = false;
      if (!accs.microsoft_available && !accs.microsoft_connected) {
        // This build has no Microsoft client ID yet (KAN-39).
        msStatus.textContent = "Outlook / Microsoft 365 support is coming soon";
        msStatus.className = "text-muted text-sm";
        btnMs.textContent = "Coming Soon";
        btnMs.className = "btn btn-secondary";
        btnMs.disabled = true;
      } else if (accs.microsoft_connected) {
        msStatus.textContent = "Connected via Microsoft Entra";
        msStatus.className = "text-success text-sm";
        btnMs.textContent = "Disconnect";
        btnMs.className = "btn btn-danger text-xs";
      } else {
        msStatus.textContent = "Not connected";
        msStatus.className = "text-muted text-sm";
        btnMs.textContent = "Connect";
        btnMs.className = "btn btn-secondary";
      }
    }

    const icsStatus = document.getElementById("ics-status");
    const btnIcs = document.getElementById("btn-connect-ics") as HTMLButtonElement | null;
    if (icsStatus && btnIcs) {
      if (accs.ics_url) {
        icsStatus.textContent = `Configured: ${accs.ics_url}`;
        icsStatus.className = "text-success text-sm";
        btnIcs.textContent = "Remove Link";
        btnIcs.className = "btn btn-danger text-xs";
      } else {
        icsStatus.textContent = "Subscribe to Apple Calendar, Fastmail, or Nextcloud";
        icsStatus.className = "text-muted text-sm";
        btnIcs.textContent = "Add Link";
        btnIcs.className = "btn btn-secondary";
      }
    }
  } catch (err) {
    console.error("Failed to query connected accounts:", err);
  }
}

function getSelectedLeadTime(): number {
  const selected = document.querySelector<HTMLInputElement>('input[name="lead-time"]:checked');
  return selected ? parseInt(selected.value, 10) || 5 : 5;
}

async function loadCalendarSyncConfig() {
  try {
    const config = await invoke<CalendarSyncConfig>("get_calendar_sync_config");
    const togglePrivate = document.getElementById("toggle-private-mode") as HTMLInputElement | null;
    const toggleAllDay = document.getElementById("toggle-skip-allday") as HTMLInputElement | null;
    const toggleTentative = document.getElementById("toggle-skip-tentative") as HTMLInputElement | null;

    if (togglePrivate) togglePrivate.checked = config.private_mode;
    if (toggleAllDay) toggleAllDay.checked = config.skip_all_day;
    if (toggleTentative) toggleTentative.checked = config.skip_tentative;

    const leadRadio = document.querySelector<HTMLInputElement>(`input[name="lead-time"][value="${config.leadTimeMin}"]`);
    if (leadRadio) leadRadio.checked = true;
  } catch (err) {
    console.warn("Failed to load sync config:", err);
  }
}

async function persistCurrentSyncConfig() {
  const togglePrivate = document.getElementById("toggle-private-mode") as HTMLInputElement | null;
  const toggleAllDay = document.getElementById("toggle-skip-allday") as HTMLInputElement | null;
  const toggleTentative = document.getElementById("toggle-skip-tentative") as HTMLInputElement | null;

  const config: CalendarSyncConfig = {
    private_mode: togglePrivate ? togglePrivate.checked : false,
    skip_all_day: toggleAllDay ? toggleAllDay.checked : true,
    skip_tentative: toggleTentative ? toggleTentative.checked : true,
    leadTimeMin: getSelectedLeadTime(),
  };

  try {
    await invoke("save_calendar_sync_config", { config });
  } catch (err) {
    console.error("Failed to save sync config:", err);
  }
}

function setupRemindersTab() {
  const btnTestChime = document.getElementById("btn-test-chime");
  if (btnTestChime) {
    btnTestChime.addEventListener("click", async () => {
      btnTestChime.setAttribute("disabled", "true");
      btnTestChime.textContent = "Chiming...";
      try {
        await invoke("test_device_chime");
      } catch (err) {
        alert("Failed to play chime: " + err);
      } finally {
        btnTestChime.removeAttribute("disabled");
        btnTestChime.textContent = "Play Test Chime";
      }
    });
  }

  // Auto-persist and re-sync whenever reminder settings change
  document.querySelectorAll<HTMLInputElement>('input[name="lead-time"]').forEach((radio) => {
    radio.addEventListener("change", async () => {
      await persistCurrentSyncConfig();
      syncAndPreviewEvents();
    });
  });

  const togglePrivate = document.getElementById("toggle-private-mode");
  const toggleAllDay = document.getElementById("toggle-skip-allday");
  const toggleTentative = document.getElementById("toggle-skip-tentative");

  [togglePrivate, toggleAllDay, toggleTentative].forEach((el) => {
    if (el) {
      el.addEventListener("change", async () => {
        await persistCurrentSyncConfig();
        syncAndPreviewEvents();
      });
    }
  });
}

function renderEventsPreview(events: CalendarEvent[]) {
  const previewContainer = document.getElementById("calendar-events-preview");
  if (!previewContainer) return;

  if (events.length === 0) {
    previewContainer.innerHTML = `<p class="text-muted text-xs py-2">No upcoming meetings found for the selected filters.</p>`;
  } else {
    previewContainer.innerHTML = "";
    events.slice(0, 10).forEach((evt) => {
      const row = document.createElement("div");
      row.className = "event-row";

      const badgeClass =
        evt.source === "google"
          ? "badge-google"
          : evt.source === "microsoft"
          ? "badge-microsoft"
          : "badge-ics";

      const sourceLabel =
        evt.source === "google"
          ? "Google"
          : evt.source === "microsoft"
          ? "Outlook"
          : "ICS";

      const timeStr = formatEventTime(evt.start, evt.end);

      row.innerHTML = `
        <div class="event-details">
          <strong>${escapeHtml(evt.title)}</strong>
          <span class="event-time">${timeStr}</span>
        </div>
        <span class="event-badge ${badgeClass}">${sourceLabel}</span>
      `;
      previewContainer.appendChild(row);
    });
  }
}

async function syncAndPreviewEvents() {
  const previewContainer = document.getElementById("calendar-events-preview");
  const btnRefresh = document.getElementById("btn-refresh-calendar") as HTMLButtonElement | null;
  if (!previewContainer) return;

  if (btnRefresh) {
    btnRefresh.disabled = true;
    btnRefresh.textContent = "Syncing...";
  }

  try {
    const togglePrivate = document.getElementById("toggle-private-mode") as HTMLInputElement | null;
    const toggleAllDay = document.getElementById("toggle-skip-allday") as HTMLInputElement | null;
    const toggleTentative = document.getElementById("toggle-skip-tentative") as HTMLInputElement | null;

    const settings: CalendarSyncSettings = {
      private_mode: togglePrivate ? togglePrivate.checked : false,
      skip_all_day: toggleAllDay ? toggleAllDay.checked : true,
      skip_tentative: toggleTentative ? toggleTentative.checked : true,
    };

    const events = await invoke<CalendarEvent[]>("sync_calendar_events", { settings });

    // Trigger immediate background sync & push to paired CubiPal (KAN-33)
    await invoke("trigger_calendar_sync");

    renderEventsPreview(events);
  } catch (err) {
    previewContainer.innerHTML = `<p class="text-danger text-xs py-2">Sync failed: ${escapeHtml(String(err))}</p>`;
  } finally {
    if (btnRefresh) {
      btnRefresh.disabled = false;
      btnRefresh.textContent = "Sync Now";
    }
  }
}

function formatEventTime(startSec: number, endSec: number): string {
  const start = new Date(startSec * 1000);
  const end = new Date(endSec * 1000);
  const now = new Date();

  const isToday =
    start.getFullYear() === now.getFullYear() &&
    start.getMonth() === now.getMonth() &&
    start.getDate() === now.getDate();

  const dateLabel = isToday
    ? "Today"
    : start.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });

  const timeStart = start.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const timeEnd = end.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

  return `${dateLabel} • ${timeStart} - ${timeEnd}`;
}

function escapeHtml(str: string): string {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

async function setupTrayListeners() {
  await listen("sync-now", () => {
    syncAndPreviewEvents();
  });

  await listen("open-device-web", async () => {
    if (currentDeviceIp) {
      await openUrl(`http://${currentDeviceIp}`);
    }
  });

  await listen<number>("start-timer", async (event) => {
    const mins = event.payload;
    try {
      await invoke("start_device_timer", { minutes: mins, label: mins > 0 ? "Focus" : "" });
      if (mins > 0) {
        alert(`Started ${mins}-minute focus timer on your CubiPal!`);
      } else {
        alert("Focus timer cancelled.");
      }
      await refreshDeviceStatus();
    } catch (e) {
      alert("Failed to start timer: " + e);
    }
  });

  await listen<CalendarEvent[]>("calendar-events-updated", (event) => {
    renderEventsPreview(event.payload);
  });

  // Desktop app self-update: tray "Check for Updates..." and the daily background check.
  await listen("check-desktop-updates", async () => {
    try {
      const info = await invoke<DesktopUpdateInfo | null>("check_desktop_update");
      await offerDesktopUpdate(info, true);
    } catch (e) {
      alert("Couldn't check for app updates: " + e);
    }
  });

  // The backend only runs the background check while Automatic Updates is on.
  await listen<DesktopUpdateInfo>("desktop-update-available", async (event) => {
    await offerDesktopUpdate(event.payload, false);
  });

  await listen<number>("snooze-alerts", () => {
    alert("Meeting reminders snoozed for 1 hour on your CubiPal.");
  });
}

// --------------------------------------------------------------------------
// General tab: Launch at Startup and Automatic Updates (TST-14)
// --------------------------------------------------------------------------

async function setupGeneralTab() {
  const autostart = document.getElementById("toggle-autostart") as HTMLInputElement | null;
  const autoUpdate = document.getElementById("toggle-autoupdate") as HTMLInputElement | null;
  const versionEl = document.getElementById("app-version");

  try {
    const s = await invoke<GeneralSettings>("get_general_settings");
    if (autostart) autostart.checked = s.autostart;
    if (autoUpdate) autoUpdate.checked = s.auto_update;
    if (versionEl) versionEl.textContent = `Version ${s.app_version}`;
  } catch (err) {
    console.warn("Failed to load general settings:", err);
  }

  autostart?.addEventListener("change", async () => {
    const wanted = autostart.checked;
    autostart.disabled = true;
    try {
      // Show what the OS actually has now.
      autostart.checked = await invoke<boolean>("set_autostart", { enabled: wanted });
    } catch (err) {
      autostart.checked = !wanted;
      alert(String(err));
    } finally {
      autostart.disabled = false;
    }
  });

  autoUpdate?.addEventListener("change", async () => {
    const wanted = autoUpdate.checked;
    try {
      await invoke("set_auto_update", { enabled: wanted });
    } catch (err) {
      autoUpdate.checked = !wanted;
      alert(String(err));
    }
  });
}

let desktopUpdatePromptOpen = false;

async function offerDesktopUpdate(info: DesktopUpdateInfo | null, manual: boolean) {
  if (!info) {
    if (manual) alert("CubiPal Desktop is up to date.");
    return;
  }
  if (desktopUpdatePromptOpen) return;
  desktopUpdatePromptOpen = true;
  try {
    await invoke("show_preferences_window");
    if (
      confirm(
        `CubiPal Desktop ${info.version} is available (you have ${info.current_version}).\n\nInstall it now? The app will restart.`
      )
    ) {
      await invoke("install_desktop_update");
    }
  } catch (e) {
    alert("App update failed: " + e);
  } finally {
    desktopUpdatePromptOpen = false;
  }
}

// --------------------------------------------------------------------------
// OTA Firmware Updates (KAN-26)
// --------------------------------------------------------------------------

async function checkFirmwareUpdate() {
  const otaCard = document.getElementById("ota-card");
  if (!otaCard) return;
  const titleEl = document.getElementById("ota-title");
  const descEl = document.getElementById("ota-desc");
  const btnInstall = document.getElementById("btn-install-ota") as HTMLButtonElement | null;

  try {
    const info = await invoke<FirmwareUpdateInfo>("check_firmware_update", { force: false });
    if (info.available && info.latest_version) {
      // An update exists. It can be installed unless the backend says why not
      // (e.g. the Pal doesn't report its board type).
      firmwareUpdateReady = !info.blocked_reason;
      if (titleEl) titleEl.textContent = `Firmware v${info.latest_version} Available`;
      if (descEl) {
        descEl.textContent = info.blocked_reason
          ? `Your Pal is running ${info.current_version}. ${info.blocked_reason}`
          : `Your Pal is running ${info.current_version}. Update over Wi-Fi; the download is checked against its published size and SHA-256 checksum.`;
      }
      if (btnInstall) btnInstall.classList.toggle("hidden", !firmwareUpdateReady);
      otaCard.classList.remove("hidden");
    } else {
      firmwareUpdateReady = false;
      otaCard.classList.add("hidden");
    }
  } catch {
    firmwareUpdateReady = false;
    otaCard.classList.add("hidden");
  }
}

function setupOta() {
  const btnInstall = document.getElementById("btn-install-ota") as HTMLButtonElement | null;
  const progressBox = document.getElementById("ota-progress-box");
  const progressBar = document.getElementById("ota-progress-bar");
  const progressText = document.getElementById("ota-progress-text");

  if (btnInstall) {
    btnInstall.addEventListener("click", async () => {
      if (!firmwareUpdateReady) return;
      if (
        !confirm(
          "Install firmware update on your CubiPal now?\n\nThe download is checked (size and SHA-256) before it is sent to your Pal. Your Pal installs it over Wi-Fi and restarts.\n\nThe update won't start while a focus timer is running or a meeting is about to start."
        )
      ) {
        return;
      }
      btnInstall.disabled = true;
      btnInstall.textContent = "Updating...";
      if (progressBox) progressBox.classList.remove("hidden");

      try {
        await invoke("install_firmware_update", { force: false });
      } catch (err) {
        // The backend names the failed stage and the Pal's own reason; keep it
        // visible in the card rather than in a passing alert (TST-19).
        if (progressText) {
          progressText.textContent = String(err);
          progressText.classList.add("text-danger");
        }
        btnInstall.disabled = false;
        btnInstall.textContent = "Retry Update";
      }
    });
  }

  listen<OtaProgress>("ota-progress", (event) => {
    const p = event.payload;
    if (progressBox) progressBox.classList.remove("hidden");
    if (progressBar) progressBar.style.width = `${p.percent}%`;
    if (progressText) {
      progressText.textContent = p.message;
      progressText.classList.toggle("text-danger", p.stage === "error");
    }

    if (p.stage === "completed") {
      if (btnInstall) {
        btnInstall.textContent = "Updated!";
      }
      setTimeout(async () => {
        if (progressBox) progressBox.classList.add("hidden");
        if (progressBar) progressBar.style.width = "0%";
        await refreshDeviceStatus();
      }, 5000);
    } else if (p.stage === "error") {
      if (btnInstall) {
        btnInstall.disabled = false;
        btnInstall.textContent = "Retry Update";
      }
    }
  });
}
