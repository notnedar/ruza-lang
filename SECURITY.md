# Security Policy & Verification Guide

## What Ruza-Lang Does (and Does NOT Do)

Ruza-Lang is a 100% open-source localization & asset-repacking tool designed for complete transparency:

- **Offline Patching:** Patching, font swapping, news translation, and rollback operate strictly on local client data files while the game is closed.
- **Zero Telemetry & Strict Update Allowlist:** The only outbound network request is an optional GitHub Releases check against `https://api.github.com/repos/notnedar/ruza-lang/releases/latest`. All update downloads enforce HTTPS-only URLs pinned to `api.github.com`, `github.com`, `objects.githubusercontent.com`, and `release-assets.githubusercontent.com` (`src/updater.rs`) and verify SHA-256 against official `SHA256SUMS.txt`.
- **No Elevation, No Root, No Injection:**
  - Windows PC: Does **not** require Administrator/UAC privileges, does **not** install kernel drivers or services, does **not** inject DLLs, and does **not** read or write game process memory. It checks running process names solely to block patching while the client is open.
  - Android APK: **100% non-root** and does **not** re-sign or modify the target APK.
- **Minimal Android Permissions (`android/app/src/main/AndroidManifest.xml`):**
  - `INTERNET` — optional update check against GitHub Releases.
  - Storage / SAF / Shizuku (`READ_EXTERNAL_STORAGE`, `WRITE_EXTERNAL_STORAGE` on Android ≤ 12, `MANAGE_EXTERNAL_STORAGE` / Shizuku UserService) — used solely to read, back up, and patch the target client's local external data bundles.
  - Zero access to contacts, SMS, camera, microphone, location, accounts, or notifications.

## Why Windows SmartScreen or Antivirus May Show a Warning

Official Windows binaries (`RuzaLangPatcher.exe`) are not signed with a paid commercial Authenticode certificate. As a result, Windows Defender SmartScreen may display an *"Unknown publisher"* warning, and a small number of heuristic antivirus engines may flag unsigned executables that modify local files.

You never have to take our word for it — every release can be independently verified:

## How to Verify Every Official Release

1. **Built Exclusively on GitHub Hosted Runners:**
   Official `RuzaLangPatcher.exe` and `RuzaLangPatcher.apk` binaries are compiled directly on GitHub's `windows-latest` and `ubuntu-latest` runners via [`.github/workflows/release.yml`](.github/workflows/release.yml) — never uploaded from a developer's local machine.
2. **Cryptographic Build Provenance (SLSA Attestation):**
   Every release artifact is signed with GitHub Artifact Build Provenance Attestation. Verify locally with the [GitHub CLI](https://cli.github.com/):
   ```bash
   gh attestation verify RuzaLangPatcher.exe --owner notnedar
   gh attestation verify RuzaLangPatcher.apk --owner notnedar
   ```
3. **SHA-256 Checksums, ClamAV CI Scan & VirusTotal:**
   - Every release publishes `SHA256SUMS.txt`, scans both binaries with **ClamAV** inside the public GitHub Actions build log before publishing, and includes direct **VirusTotal** SHA-256 report links in the Release Notes.
4. **Automated Continuous Security Checks:**
   - **GitHub CodeQL** static analysis, **Dependabot** automated dependency updates, **dependency advisory audits**, and **OpenSSF Scorecard** run automatically on every push and weekly schedule.
5. **Audit All Embedded Data Payloads Locally:**
   Verify that every compressed file under `resources/` matches the plain-text JSON dictionaries in `patches/` and official OFL-1.1 Barlow Semi Condensed font files (zero executable headers):
   ```bash
   node tools/verify-resources.mjs
   ```
6. **Build from Source:**
   If you prefer not to run pre-built binaries at all, follow the **Build from Source** instructions in [`README.md`](README.md).

## Reporting a Vulnerability

If you discover a security issue in Ruza-Lang:
- Please report it privately via **GitHub Security Advisories** (`Security → Advisories → Report a vulnerability`) or open a minimal issue requesting private contact without posting exploit details publicly.
- Supported versions: the latest `0.9.x` release line.
