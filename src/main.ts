// Tab switching logic
document.addEventListener("DOMContentLoaded", () => {
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

  // Action button placeholders
  const btnScan = document.getElementById("btn-scan");
  btnScan?.addEventListener("click", () => {
    alert("Network scanning and mDNS pairing wizard will activate in KAN-29.");
  });

  const btnConnectGoogle = document.getElementById("btn-connect-google");
  btnConnectGoogle?.addEventListener("click", () => {
    alert("Google Calendar OAuth 2.0 PKCE sign-in will activate in KAN-30.");
  });

  const btnConnectMs = document.getElementById("btn-connect-ms");
  btnConnectMs?.addEventListener("click", () => {
    alert("Microsoft 365 / Outlook OAuth 2.0 PKCE sign-in will activate in KAN-31.");
  });

  const btnConnectIcs = document.getElementById("btn-connect-ics");
  btnConnectIcs?.addEventListener("click", () => {
    const url = prompt("Enter private iCal / ICS feed URL:");
    if (url) {
      alert("ICS calendar URL saved (stubbed for KAN-32).");
    }
  });

  // Privacy and lead time preferences
  const togglePrivateMode = document.getElementById("toggle-private-mode") as HTMLInputElement | null;
  togglePrivateMode?.addEventListener("change", (e) => {
    const checked = (e.target as HTMLInputElement).checked;
    console.log("Private Mode:", checked);
  });
});
