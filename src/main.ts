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
  skin?: string;
  ip: string;
  connected: boolean;
}

interface DiscoveredPal {
  id: string;
  name: string;
  ip: string;
  port: number;
  model: string;
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
  ics_url: string | null;
}

interface CalendarSyncSettings {
  private_mode: boolean;
  skip_all_day: boolean;
  skip_tentative: boolean;
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
  latest_version: string;
  current_version: string;
  download_url: string | null;
  release_notes: string | null;
}

interface OtaProgress {
  stage: string;
  percent: number;
  message: string;
}

let currentDeviceIp: string | null = null;
let activeFirmwareUrl: string | null = null;

document.addEventListener("DOMContentLoaded", async () => {
  setupTabs();
  setupPairingModal();
  setupActionButtons();
  setupCalendarTab();
  setupRemindersTab();
  setupTrayListeners();
  setupOta();

  // Load initial paired device status and calendars
  await refreshDeviceStatus();
  await refreshConnectedAccounts();
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
      fwEl.textContent = dev.fw || "v1.0.0";
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
      await checkFirmwareUpdate();
    } else if (dev && !dev.connected) {
      document.getElementById("ota-card")?.classList.add("hidden");
      currentDeviceIp = dev.ip;
      nameEl.textContent = `${dev.robot} (Offline)`;
      subtitleEl.textContent = "Cannot reach device on the network";
      ipEl.textContent = `http://${dev.ip} (Unreachable)`;
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
      currentDeviceIp = null;
      nameEl.textContent = "No CubiPal Paired";
      subtitleEl.textContent = "Put your Pal into pairing mode to begin";
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
        pals.forEach((pal) => {
          const row = document.createElement("div");
          row.className = "discovered-row";
          row.innerHTML = `
            <div class="discovered-info">
              <strong>${pal.name}</strong>
              <div class="discovered-meta">${pal.ip} &bull; Model ${pal.model} &bull; FW ${pal.fw}</div>
            </div>
            <button class="btn btn-primary btn-pair-pal" data-ip="${pal.ip}" data-name="${pal.name}">Pair</button>
          `;
          scanList.appendChild(row);
        });

        scanList.querySelectorAll<HTMLButtonElement>(".btn-pair-pal").forEach((btn) => {
          btn.addEventListener("click", () => {
            const ip = btn.getAttribute("data-ip")!;
            const name = btn.getAttribute("data-name")!;
            openPairingModal(ip, name);
          });
        });
      }
    } catch (e) {
      scanTitle.textContent = "Scan Error";
      scanList.innerHTML = `<p class="text-muted text-sm">${e}</p>`;
    }
  });

  const btnManualIp = document.getElementById("btn-manual-ip")!;
  btnManualIp.addEventListener("click", () => {
    const ip = prompt("Enter CubiPal IP address or hostname (e.g. 192.168.1.100 or cubipal-ab12.local):");
    if (ip && ip.trim()) {
      openPairingModal(ip.trim(), "CubiPal");
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
  const errorMsg = document.getElementById("pairing-error-msg")!;

  const closeModal = () => {
    modal.classList.add("hidden");
    inputCode.value = "";
    errorMsg.classList.add("hidden");
    btnPair.disabled = false;
  };

  btnClose.addEventListener("click", closeModal);
  btnCancel.addEventListener("click", closeModal);

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
    const code = inputCode.value.trim();

    if (code.replace(/\s/g, "").length !== 8) {
      errorMsg.textContent = "Please enter all 8 digits shown on your CubiPal.";
      errorMsg.classList.remove("hidden");
      return;
    }

    btnPair.disabled = true;
    errorMsg.classList.add("hidden");

    try {
      const res = await invoke<PairResponse>("pair_pal", { ip, code });
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

function openPairingModal(ip: string, name: string) {
  const modal = document.getElementById("pairing-modal")!;
  const title = document.getElementById("modal-pal-name")!;
  const targetIpInput = document.getElementById("modal-target-ip") as HTMLInputElement;
  const inputCode = document.getElementById("input-pairing-code") as HTMLInputElement;
  const errorMsg = document.getElementById("pairing-error-msg")!;

  title.textContent = `Pair with ${name}`;
  targetIpInput.value = ip;
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
      if (accs.microsoft_connected) {
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

  document.querySelectorAll<HTMLInputElement>('input[name="lead-time"]').forEach((radio) => {
    radio.addEventListener("change", () => {
      syncAndPreviewEvents();
    });
  });
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

    // Push next events to paired CubiPal (KAN-33)
    try {
      await invoke("push_events_to_device", {
        events,
        leadTimeMin: getSelectedLeadTime(),
      });
    } catch (pushErr) {
      console.warn("Event push to paired CubiPal failed or device offline:", pushErr);
    }

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

  await listen<number>("snooze-alerts", () => {
    alert("Meeting reminders snoozed for 1 hour on your CubiPal.");
  });
}

// --------------------------------------------------------------------------
// OTA Firmware Updates (KAN-26)
// --------------------------------------------------------------------------

async function checkFirmwareUpdate() {
  const otaCard = document.getElementById("ota-card");
  if (!otaCard) return;

  try {
    const info = await invoke<FirmwareUpdateInfo>("check_firmware_update");
    if (info && info.available && info.download_url) {
      activeFirmwareUrl = info.download_url;
      const titleEl = document.getElementById("ota-title");
      const descEl = document.getElementById("ota-desc");
      if (titleEl) titleEl.textContent = `Firmware v${info.latest_version} Available`;
      if (descEl) descEl.textContent = `Your Pal is running v${info.current_version}. Update directly over Wi-Fi.`;
      otaCard.classList.remove("hidden");
    } else {
      otaCard.classList.add("hidden");
    }
  } catch {
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
      if (!activeFirmwareUrl) return;
      if (
        !confirm(
          "Install firmware update on your CubiPal now?\n\nYour Pal will flash the update over Wi-Fi and restart automatically."
        )
      ) {
        return;
      }
      btnInstall.disabled = true;
      btnInstall.textContent = "Updating...";
      if (progressBox) progressBox.classList.remove("hidden");

      try {
        await invoke("install_firmware_update", { downloadUrl: activeFirmwareUrl });
      } catch (err) {
        alert("Firmware update failed: " + err);
        btnInstall.disabled = false;
        btnInstall.textContent = "Retry Update";
      }
    });
  }

  listen<OtaProgress>("ota-progress", (event) => {
    const p = event.payload;
    if (progressBox) progressBox.classList.remove("hidden");
    if (progressBar) progressBar.style.width = `${p.percent}%`;
    if (progressText) progressText.textContent = p.message;

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
