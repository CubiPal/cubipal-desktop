# Jira Ticket KAN-38 Status & Action Record

## Ticket Details
* **Key:** `KAN-38`
* **Summary:** Google Cloud Project & Desktop OAuth Client (`calendar.readonly`)
* **Epic:** `KAN-6` ([Software] ESP32 Firmware, Personalities & Local Web Control) / Subtask of `KAN-35`
* **Current Status:** In Progress / Blocked on External Policy
* **Due Date:** **2026-12-01** (30 days from creation, awaiting YouTube account age maturation)

---

## Completed Configuration (2026-11-01)
1. **Google Cloud Platform Project & Credentials:**
   * **Project Owner:** `mark@cubipal.com`
   * **Client ID:** `772128439990-pgutm29ngflnbdplo9khmblprs7c0m83.apps.googleusercontent.com`
   * **Client Secret:** *(Secured in GitHub Secrets `GOOGLE_CLIENT_SECRET`)*
   * **OAuth Scope:** `https://www.googleapis.com/auth/calendar.readonly`
2. **Repository & CI Integration:**
   * Set GitHub Actions Secrets `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in [`houndman1017/cubipal-desktop`](https://github.com/houndman1017/cubipal-desktop).
   * Updated `GOOGLE_DEFAULT_CLIENT_ID` constant in `src-tauri/src/calendar/google.rs`.
   * Updated `.github/workflows/release.yml` build environment.
3. **Verification Materials Prepared:**
   * Automated 1080p verification video generator studio built at `docs/oauth-verification-video-studio.html`.
   * Pre-formatted YouTube title, description, timestamped chapters, and Google Trust & Safety scope justification ready.

---

## Blocker & Next Actions on 2026-12-01
* **Blocker:** YouTube restricts new accounts from uploading unlisted/public verification videos until the account is at least 30 days old.
* **Scheduled Action (2026-12-01):**
  1. Open `docs/oauth-verification-video-studio.html` and click **Render & Download Video (.webm)**.
  2. Upload `cubipal-oauth-verification-demo-KAN-38.webm` to YouTube as **Unlisted**.
  3. Submit the unlisted YouTube link in Google Cloud Console (`console.cloud.google.com/apis/credentials/consent`) for final verification review.
