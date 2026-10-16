# Security policy

ClipMac handles sensitive data — your clipboard and screenshots — so security and privacy reports are taken seriously.

## Reporting a vulnerability

**Please don't open a public issue.** Use GitHub's private reporting instead: on the repository page go to **Security → Report a vulnerability**. Include what you found, how to reproduce it, and the macOS and ClipMac versions.

You can expect an acknowledgement within a few days. Fixes are released as a new version, and reporters are credited unless they prefer otherwise.

## What counts

Examples of things worth reporting:

- A way for secrets that ClipMac should be hiding to leak while a clip is hidden — via the preview, list, menu bar, drag-and-drop, export, copy, or temporary files.
- Clipboard or screen content leaving the device (ClipMac is designed to make **no network requests**).
- Another process or website being able to read or modify the stored history.
- Anything that executes unexpected code, for example from crafted clipboard content.

## Known limitations (not vulnerabilities)

These are documented, deliberate trade-offs:

- Clipboard history is stored **unencrypted** in `~/Library/Application Support/com.clipmac.app/` and anything not favorited is deleted after 24 hours.
- Secret detection is **best effort**. OCR can miss small or stylized text and the detector knows common formats only. A missed secret is a bug worth reporting as a detection improvement, but it is not a vulnerability.
- Release builds are currently ad-hoc signed and not notarized.

## Supported versions

Only the latest release receives fixes.
