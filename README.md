# ClipMac

A clipboard manager and screenshot tool for macOS. It lives in your menu bar, remembers what you copy for 24 hours, hides secrets automatically, and lets you capture, beautify and share screenshots.

Built with [Tauri 2](https://tauri.app) (Rust) and a plain HTML/CSS/JS frontend. No Node, no bundler, no network access.

**Version 1.0.1 · macOS only**

## Features

**Clipboard history**
- Saves everything you copy: **text** and **images**, grouped into *Text* and *Images*.
- Everything is deleted after **24 hours**. Mark a clip as a **favorite** to keep it; favorites never expire and can't be deleted or cleared until you unfavorite them.
- Search, filter (All / Favorites / Text / Images) and sort (Newest, Oldest, Largest, Smallest).
- Menu-bar icon listing your latest copies — click one to copy it again. Closing the window keeps the app running.
- Deleting and clearing always ask for confirmation.

**Secrets are hidden automatically**
- API keys, tokens, passwords, JWTs, private keys and credentials in URLs are detected in copied text and — using on-device OCR — inside screenshots.
- Hidden clips show `••••••••` (text) or black bars (images) until you click **Show**. Drag, export, copy and the menu bar all use the redacted version while hidden.
- Best effort, not a guarantee — see [Privacy](#privacy-and-security).

**Screenshots**
- Capture a **selection**, a **window** or the **full screen** at full native (Retina) resolution, as lossless PNG.
- Options: delay, show cursor, window shadow, and an optional copy saved to `~/Pictures/ClipMac Screenshots`.
- **Global keyboard shortcuts** for each capture mode, set in the Screenshot panel.

**Image editor** (Xnapper-style)
- Background (gradients, solid colors, custom color, or transparent), padding, corner radius, shadow, zoom + pan, and aspect ratio.
- **Save Copy** or **Overwrite**, saved as a favorite by default so it doesn't expire.

**Images as files**
- Drag an image straight into Finder, Mail, Slack, Figma or a browser.
- **Copy as File**, or **Export** to PNG / JPEG.

**Code**
- Code is detected automatically and shown with syntax highlighting and line numbers.
- A **Format** button tidies JavaScript, TypeScript, JSON, CSS, SCSS, Less, HTML, Markdown, YAML (via Prettier) and Rust (via `rustfmt`). Copy the formatted version without changing the saved clip.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `↑` `↓` | Move through clips |
| `⏎` | Copy the selected clip |
| `⌘F` or `/` | Search |
| `Esc` | Clear search / close panels |
| `⌘D` | Favorite / unfavorite |
| `⌘E` | Edit the selected image |
| `⌘⌫` | Delete the selected clip |

Screenshot shortcuts are yours to choose: open the camera panel → **Keyboard Shortcuts** → click a field and press the keys. Avoid `⌘⇧3/4/5`, which macOS reserves.

## Getting started

**Requirements:** macOS, [Rust](https://rustup.rs) (stable, 1.85+), Xcode Command Line Tools (`xcode-select --install`).

```sh
# one-time: the Tauri CLI
cargo install tauri-cli --version "^2" --locked

# run in development
cd src-tauri
cargo tauri dev
```

### Permissions

The first screenshot asks for **Screen Recording** (System Settings → Privacy & Security). Enable it, then **restart the app**.

> When running with `cargo tauri dev`, macOS grants the permission to the **terminal app** you launched it from (Terminal, iTerm, VS Code…), not to ClipMac. Without it, captures show only your wallpaper.

### Build

```sh
cd src-tauri
cargo tauri build                                   # for this Mac
cargo tauri build --target universal-apple-darwin   # Apple Silicon + Intel
                                                    # (needs: rustup target add aarch64-apple-darwin x86_64-apple-darwin)
```

The `.app` and `.dmg` appear under `src-tauri/target/<target>/release/bundle/`.

### Tests

```sh
cd src-tauri
cargo test
```

Covers secret detection and masking, shortcut parsing and Rust formatting. An optional end-to-end OCR test runs when `SECRETS_TEST_PNG` and `SECRETS_TEST_OUT` point at an input image and an output path.

## Continuous integration

[`.github/workflows/build-macos.yml`](.github/workflows/build-macos.yml) tests and builds a universal macOS app on every push to `main`, every pull request and on demand, and uploads the `.dmg` / `.app.zip` as a run artifact. Pushing a tag such as `v1.0.0` also creates a **draft GitHub release** with the downloads.

Builds are **ad-hoc signed, not notarized**. Downloaded copies trigger a Gatekeeper warning (right-click → Open, or `xattr -cr ClipMac.app`), and since the signature changes with every build, macOS asks for Screen Recording again after each update. Signing with a Developer ID certificate fixes both — the needed secrets are listed (commented out) in the workflow.

## Privacy and security

- **Everything stays on your Mac.** The app makes no network requests; its libraries are bundled in `src/vendor`.
- History is stored **unencrypted** in `~/Library/Application Support/com.clipmac.app/` (`clips.json`, `settings.json`, `images/`). Anything not favorited is deleted after 24 hours.
- Secret detection is **best effort**. OCR can miss small, blurry or stylized text, and the detector knows common formats, not every possible one. Don't rely on it as your only protection. It doesn't cover credit card numbers or personal data such as emails and phone numbers.
- Redacted images black out the whole word containing a secret (`API_KEY=abc…` hides the key name too) — over-hiding by design.
- Clips captured before auto-hide was switched on (or while it's off) aren't scanned.
- Uses Tauri's `macos-private-api` for the transparent, vibrant window, so the app isn't eligible for the Mac App Store.

## Project layout

```
src/                    Frontend (no build step)
  index.html            Markup
  styles.css            Styling (light/dark, system accent color)
  main.js               App logic: list, preview, search, shortcuts, screenshots, dialogs
  editor.js             Image editor (canvas rendering + export)
  code.js               Code detection, highlighting, formatting
  vendor/               highlight.js and Prettier, bundled for offline use
src-tauri/
  src/lib.rs            Clipboard watcher, storage, tray, screenshots, shortcuts, commands
  src/secrets.rs        Secret detection for text
  src/ocr.rs            Screenshot OCR via the macOS Vision framework
  tauri.conf.json       Window, vibrancy, bundle settings
  icons/                App and menu-bar icons
.github/                Workflows, issue and PR templates
LICENSE, SECURITY.md, CONTRIBUTING.md, THIRD_PARTY_LICENSES.md
```

## Third-party code

- [highlight.js](https://highlightjs.org) 11.9.0 — BSD-3-Clause
- [Prettier](https://prettier.io) 3.3.3 — MIT
- [Tauri](https://tauri.app), [arboard](https://github.com/1Password/arboard), [`drag`](https://crates.io/crates/drag) and the [`objc2`](https://github.com/madsmtm/objc2) crates — see [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Found a security problem? Please read [SECURITY.md](SECURITY.md) first instead of opening a public issue.

## License

[MIT](LICENSE) © ClipMac contributors. Third-party notices are in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
