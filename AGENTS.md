# CubiPal — Shared Agent & Ecosystem Configuration

This file is shared between **Google Antigravity** (`AGENTS.md`) and **Claude Code** (`CLAUDE.md`) so both agents operate on the same repositories, project management boards, and documentation.

## 1. Project Overview & Repositories

- **Core Product Repo (Firmware + 3D Cases + Docs):** `CubiPal` (GitHub: `git@github.com:CubiPal/CubiPal.git`)
  - Product brief: `docs/product-direction.md`
  - Firmware (`ESP32` Arduino): `firmware/CubiPal/` (build with `firmware/build.sh`)
  - 3D Enclosures / Skins (OpenSCAD): `case/src/` (export with `bash case/export-all.sh`, handoff notes in `case/docs/cad-handoff.md`)
  - CubiCore v6 specs & software tickets: `docs/cubicore-v6-software-tickets.md` (`KAN-40`, `KAN-41`, `KAN-42`)
- **Desktop Companion App Repo (Tauri v2 + Rust + Web):** `cubipal-desktop` (GitHub: `git@github.com:CubiPal/cubipal-desktop.git`)
  - Architecture: Tauri v2 system tray app, local mDNS discovery, HTTP REST to CubiPal device
  - Calendar Sync: Google Calendar OAuth 2.0 PKCE (`KAN-38`), Outlook / Microsoft 365 OAuth (`KAN-39`), iCal ICS
  - GitHub Actions Secrets: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` configured for release builds
- **Marketing & Website Repo:** `cubipal-website` (GitHub: `git@github.com:CubiPal/cubipal-website.git`)
  - Cloudflare Worker (`cubipal-website`) serving `cubipal.com` and `www.cubipal.com`
  - D1 Database: `cubipal-waitlist`
  - Turnstile Widget: `cubipal.com waitlist`

*Local paths:* Windows: `C:\Users\markb\Projects\<repo>` | WSL/Linux: `/home/houndman1017/Projects/<repo>`

## 2. Connected Ecosystem & Tooling (Shared Across Antigravity & Claude Code)

### A. Primary Identity & Email Stack
- **Domain:** `cubipal.com` (Hosted on Cloudflare DNS, Google Workspace for mail)
- **Primary Engineering & Admin Identity:** `Mark <mark@cubipal.com>`
  - Git Author (CubiPal org repos): `user.name "CubiPal"`, `user.email "mark@cubipal.com"` (verified on GitHub)
- **Active Email Aliases (All route to `mark@cubipal.com`):**
  - `admin@cubipal.com` — Receives automated DMARC XML security and deliverability reports
  - `privacy@cubipal.com` — Legal / privacy contact point referenced in `cubipal-website/privacy.html`
  - `support@cubipal.com` & `hello@cubipal.com` — Customer inquiries and feedback
- **DNS Email Authentication:**
  - **MX:** `1 smtp.google.com`
  - **SPF:** `v=spf1 include:_spf.google.com ~all`
  - **DKIM:** `google._domainkey.cubipal.com` (2048-bit RSA)
  - **DMARC:** `_dmarc.cubipal.com` (`v=DMARC1; p=none; rua=mailto:admin@cubipal.com`)

### B. Jira / Atlassian (`cubipal.atlassian.net`)
- **Purpose:** Primary project & engineering tracking across **Hardware**, **3D Design**, **Software/Firmware**, **Web/E-Commerce**, and **Business/Operations**.
- **Administrators:** `mark@cubipal.com`, `houndman1017@gmail.com`
- **CLI Binary:** `twg` (run with `TWG_AGENT_DEFAULTS=1 twg ...`)
- **Atlassian Site:** `cubipal.atlassian.net` (`ca1eeb1f-3f4c-46ed-b44a-b328107bbf67`)
- **Project Key:** `KAN` (`CubiPal` — `https://cubipal.atlassian.net/jira/software/projects/KAN/boards`)
  - **Epics:**
    - `KAN-4`: `[Hardware] Core Electronics, USB-C Foot & Custom PCBA`
    - `KAN-5`: `[3D Design] Core v4, 4N Weighted Foot & 5-Panel Magnetic Skins`
    - `KAN-6`: `[Software] ESP32 Firmware, Personalities & Local Web Control`
    - `KAN-7`: `[Web & E-Commerce] cubipal.com Worker, Waitlist & Storefront`
- **Shared Skills:**
  - Antigravity: `twg`, `twg-jira`, `twg-confluence`, `twg-setup`
  - Claude Code: `twg`, `twg-jira`, `twg-confluence`, `twg-setup`

### C. Google Drive & Google Cloud Platform
- **Google Drive (`gdrive` CLI):** Source of truth for business plan, execution roadmap, and exported briefings.
  - `CubiPal` Folder: `1MA2ZXBAy4fWixQkYYND3uuyIh0ZdWBDo`
  - `CubiPal Business Plan` (Google Doc): `18RI-HmneBjM6CuxU8thSloOh5uOjamEadHBLhxh4lmQ`
  - `CubiPal Execution Roadmap` (Google Doc): `1UL5VW_k2okhF6sexFKHHwbGLtVRKbjYGH41VQKZI0xQ`
- **Google Cloud Platform (GCP Console):**
  - Project Owner: `mark@cubipal.com`
  - OAuth 2.0 Client (Desktop Companion Google Calendar Sync `KAN-38`):
    - Client ID: `772128439990-pgutm29ngflnbdplo9khmblprs7c0m83.apps.googleusercontent.com`
    - Stored in `cubipal-desktop` repository GitHub Secrets (`GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`)

### D. GitHub (`gh` CLI)
- **Account:** `houndman1017` is a member/admin of the `CubiPal` org; repos live under the org (Git commit author for new commits: `CubiPal <mark@cubipal.com>`)
- **Repositories:**
  - `CubiPal/CubiPal`
  - `CubiPal/cubipal-desktop`
  - `CubiPal/cubipal-website`
- **Convention:** Reference Jira issue keys (e.g., `KAN-40`) in branch names and commit messages to keep GitHub and Jira synchronized.

### E. Cloudflare (`cloudflare` MCP & `wrangler` CLI)
- **Account:** `mark@cubipal.com` / `houndman1017@gmail.com` (`ca91fec7529a01760cb9325370d08f84`)
- **Credentials:** `~/.wrangler/config/default.toml`
- **MCP Servers:**
  - `cloudflare`: `npx -y @cloudflare/mcp-server-cloudflare run ca91fec7529a01760cb9325370d08f84`
  - `cloudflare-docs`: `https://docs.mcp.cloudflare.com/mcp`
- **Production Resources (`Projects/cubipal-website`):**
  - **Worker:** `cubipal-website` (`cubipal.com`, `www.cubipal.com`)
  - **D1 Database:** `cubipal-waitlist` (`2df2446d-6abe-4a07-b5b2-a8c56205e9c4`)
  - **Turnstile Widget:** `cubipal.com waitlist` (secret stored in Worker secret `TURNSTILE_SECRET`)

