import { invoke } from "@tauri-apps/api/core";
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

let currentDeviceIp: string | null = null;

document.addEventListener("DOMContentLoaded", async () => {
  setupTabs();
  setupPairingModal();
  setupActionButtons();

  // Load initial paired device status
  await refreshDeviceStatus();

  // Poll state every 15 seconds
  setInterval(refreshDeviceStatus, 15000);
});

function setupTabs() {
  const navItems = document.querySelectorAll<HTMLButtonElement>(".nav-item");
  const tabPanes = document.querySelectorAll<HTMLElement>(".tab-pane");

  navItems.forEach((btn) => {
    btn.addEventListener("click", () => {
      const targetTab = btn.getAttribute("data-tab");
      if (!targetTab) return;

      navItems.forEach((item) => item.classList.remove("active"));
      tabPanes.forEach((pane) => pane.classList.remove("active"));

      btn.classList.add("active");
      const activePane = document.getElementById(`tab-${targetTab}`);
      if (activePane) {
        activePane.classList.add("active");
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

      pill.className = "status-pill online";
      pillText.textContent = "Online";

      btnOpenWeb.disabled = false;
      btnUnpair.disabled = false;
      if (btnTestChime) btnTestChime.disabled = false;
    } else if (dev && !dev.connected) {
      currentDeviceIp = dev.ip;
      nameEl.textContent = `${dev.robot} (Offline)`;
      subtitleEl.textContent = "Cannot reach device on the network";
      ipEl.textContent = `http://${dev.ip} (Unreachable)`;
      wifiEl.textContent = "Offline";
      fwEl.textContent = "—";
      volumeEl.textContent = "—";

      pill.className = "status-pill offline";
      pillText.textContent = "Offline";

      btnOpenWeb.disabled = true;
      btnUnpair.disabled = false;
      if (btnTestChime) btnTestChime.disabled = true;
    } else {
      currentDeviceIp = null;
      nameEl.textContent = "No CubiPal Paired";
      subtitleEl.textContent = "Put your Pal into pairing mode to begin";
      ipEl.textContent = "Not connected";
      wifiEl.textContent = "—";
      fwEl.textContent = "—";
      volumeEl.textContent = "—";

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
