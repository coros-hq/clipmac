mod ocr;
mod secrets;

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use arboard::{Clipboard, ImageData};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Clips older than this are deleted.
const TTL_MS: u64 = 24 * 60 * 60 * 1000;
const POLL: Duration = Duration::from_millis(500);
const PRUNE_EVERY: Duration = Duration::from_secs(30);
/// Ignore pathological text copies (e.g. whole log files).
const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;
/// How many recent clips the menu-bar menu lists.
const MENU_CLIPS: usize = 8;
const TRAY_ID: &str = "main";

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Text,
    Image,
}

#[derive(Clone, Serialize, Deserialize)]
struct Clip {
    /// Hex of the content hash, so identical content always maps to one clip.
    id: String,
    kind: Kind,
    text: Option<String>,
    width: u32,
    height: u32,
    /// Size of the content in bytes (text bytes, or encoded PNG bytes).
    bytes: u64,
    /// Capture time, ms since the Unix epoch.
    at: u64,
    /// Favorites never expire and can't be deleted or cleared until unfavorited.
    #[serde(default)]
    favorite: bool,
    /// Secrets were detected (API keys, passwords…). Hidden until the user reveals them.
    #[serde(default)]
    sensitive: bool,
    /// The user chose to show this clip's secrets.
    #[serde(default)]
    revealed: bool,
    /// What was found, e.g. ["API key", "Password"].
    #[serde(default)]
    findings: Vec<String>,
}

/// Global keyboard shortcuts for taking screenshots, as accelerator strings
/// (e.g. "Control+Alt+KeyS"). `None` = not set.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Shortcuts {
    selection: Option<String>,
    window: Option<String>,
    screen: Option<String>,
}

/// App settings, persisted in `settings.json`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    /// Hide detected secrets until the user reveals them.
    auto_hide: bool,
    /// Seconds to wait before capturing (0 = none).
    delay: u32,
    /// Include the mouse cursor (full-screen captures only).
    cursor: bool,
    /// Keep the drop shadow around captured windows.
    shadow: bool,
    /// Also save a PNG copy to ~/Pictures/ClipMac Screenshots.
    save_file: bool,
    shortcuts: Shortcuts,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_hide: true,
            delay: 0,
            cursor: false,
            shadow: true,
            save_file: false,
            shortcuts: Shortcuts::default(),
        }
    }
}

struct Store {
    dir: PathBuf,
    /// Newest first.
    clips: Vec<Clip>,
    last_change: isize,
    settings: Settings,
}

struct Shared(Mutex<Store>);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn hash_of(data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    data.hash(&mut h);
    h.finish()
}

#[cfg(target_os = "macos")]
fn change_count() -> isize {
    use objc2_app_kit::NSPasteboard;
    NSPasteboard::generalPasteboard().changeCount()
}

#[cfg(not(target_os = "macos"))]
fn change_count() -> isize {
    0
}

impl Store {
    fn load(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("images"));
        let clips = fs::read(dir.join("clips.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let settings = fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Start from the current pasteboard state so pre-existing content isn't captured.
        let _ = fs::remove_dir_all(std::env::temp_dir().join("clipmac-files")); // stale staged copies
        let mut store = Store {
            dir,
            clips,
            last_change: change_count(),
            settings,
        };
        store.prune();
        store
    }

    fn image_path(&self, id: &str) -> PathBuf {
        self.dir.join("images").join(format!("{id}.png"))
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.clips) {
            let _ = fs::write(self.dir.join("clips.json"), json);
        }
    }

    fn save_settings(&self) {
        if let Ok(json) = serde_json::to_vec(&self.settings) {
            let _ = fs::write(self.dir.join("settings.json"), json);
        }
    }

    fn redacted_path(&self, id: &str) -> PathBuf {
        self.dir.join("images").join(format!("{id}.redacted.png"))
    }

    /// The image file that may be shown or shared: the redacted copy while secrets are hidden.
    fn visible_path(&self, c: &Clip) -> Result<PathBuf, String> {
        if !is_hidden(c) {
            return Ok(self.image_path(&c.id));
        }
        let redacted = self.redacted_path(&c.id);
        if redacted.exists() {
            Ok(redacted)
        } else {
            Err("redacted copy unavailable".into())
        }
    }

    /// The image file that may leave the app (clipboard, drag, staged file). Unlike
    /// `visible_path`, revealing a clip doesn't change it: a sensitive clip always goes out as
    /// its redacted copy, and the full image only ever stays in the private store.
    fn public_path(&self, c: &Clip) -> Result<PathBuf, String> {
        if !c.sensitive {
            return Ok(self.image_path(&c.id));
        }
        let redacted = self.redacted_path(&c.id);
        if redacted.exists() {
            Ok(redacted)
        } else {
            Err("redacted copy unavailable".into())
        }
    }

    fn write_redacted(&self, id: &str, redacted: Option<&[u8]>) {
        match redacted {
            Some(bytes) => {
                let _ = fs::write(self.redacted_path(id), bytes);
            }
            None => {
                let _ = fs::remove_file(self.redacted_path(id));
            }
        }
    }

    fn drop_files(&self, clip: &Clip) {
        if clip.kind == Kind::Image {
            let _ = fs::remove_file(self.image_path(&clip.id));
            let _ = fs::remove_file(self.redacted_path(&clip.id));
            drop_staged(&clip.id);
        }
    }

    /// Removes expired clips. Returns true if anything was removed.
    fn prune(&mut self) -> bool {
        let cutoff = now_ms().saturating_sub(TTL_MS);
        let (keep, expired): (Vec<_>, Vec<_>) = self
            .clips
            .drain(..)
            .partition(|c| c.favorite || c.at >= cutoff);
        self.clips = keep;
        for c in &expired {
            self.drop_files(c);
        }
        if !expired.is_empty() {
            self.save();
        }
        !expired.is_empty()
    }

    fn insert(&mut self, p: Pending) {
        let Pending {
            mut clip,
            png,
            redacted,
        } = p;
        if let Some(i) = self.clips.iter().position(|c| c.id == clip.id) {
            // Re-copied: bubble to the top with a fresh timestamp, keeping the user's choices.
            let old = self.clips.remove(i);
            clip.favorite |= old.favorite;
            clip.revealed = old.revealed && clip.sensitive;
        }
        if let Some(png) = png {
            let _ = fs::write(self.image_path(&clip.id), png);
        }
        if clip.kind == Kind::Image {
            self.write_redacted(&clip.id, redacted.as_deref());
        }
        self.clips.insert(0, clip);
        self.save();
    }

    /// Deletes a clip. Favorites are refused; returns whether anything was deleted.
    fn remove(&mut self, id: &str) -> bool {
        let Some(i) = self.clips.iter().position(|c| c.id == id && !c.favorite) else {
            return false;
        };
        let c = self.clips.remove(i);
        self.drop_files(&c);
        self.save();
        true
    }

    /// Deletes everything except favorites.
    fn clear(&mut self) {
        let (keep, gone): (Vec<_>, Vec<_>) = self.clips.drain(..).partition(|c| c.favorite);
        self.clips = keep;
        for c in &gone {
            self.drop_files(c);
        }
        self.save();
    }

    fn set_favorite(&mut self, id: &str, favorite: bool) {
        if let Some(c) = self.clips.iter_mut().find(|c| c.id == id) {
            c.favorite = favorite;
            self.save();
        }
    }

    fn set_revealed(&mut self, id: &str, revealed: bool) {
        if let Some(c) = self.clips.iter_mut().find(|c| c.id == id && c.sensitive) {
            c.revealed = revealed;
            self.save();
        }
        if !revealed {
            drop_staged(id); // don't leave an unredacted copy lying around
        }
    }
}

fn is_hidden(c: &Clip) -> bool {
    c.sensitive && !c.revealed
}

fn drop_staged(id: &str) {
    let _ = fs::remove_dir_all(std::env::temp_dir().join("clipmac-files").join(id));
}

/// What the frontend / menu may see: hidden text has its secrets masked before it leaves Rust.
fn view(c: &Clip) -> Clip {
    let mut v = c.clone();
    if is_hidden(c) && c.kind == Kind::Text {
        v.text = v.text.map(|t| secrets::mask_text(&t));
    }
    v
}

/// A clip that has been read and analyzed but not stored yet.
struct Pending {
    clip: Clip,
    png: Option<Vec<u8>>,
    /// The image with secrets blacked out (only when something was found).
    redacted: Option<Vec<u8>>,
}

impl Pending {
    /// Bytes that are safe to put on the clipboard or write to disk outside the app.
    fn safe_png(&self) -> Option<&[u8]> {
        if self.clip.sensitive {
            self.redacted.as_deref()
        } else {
            self.png.as_deref()
        }
    }
}

/// Blacks out each hit (with a little padding) and re-encodes as PNG.
fn redact_png(png: &[u8], hits: &[ocr::Hit]) -> Option<Vec<u8>> {
    let mut img = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = (img.width() as f64, img.height() as f64);
    for hit in hits {
        let [x, y, bw, bh] = hit.rect;
        let (pad_x, pad_y) = (6.0, bh * h * 0.2);
        let (x0, x1) = (
            ((x * w) - pad_x).max(0.0) as u32,
            (((x + bw) * w) + pad_x).min(w) as u32,
        );
        let (y0, y1) = (
            ((y * h) - pad_y).max(0.0) as u32,
            (((y + bh) * h) + pad_y).min(h) as u32,
        );
        for py in y0..y1 {
            for px in x0..x1 {
                img.put_pixel(px, py, image::Rgba([24, 24, 26, 255]));
            }
        }
    }
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// OCRs the image for secrets. Returns what was found and a redacted copy.
fn scan_image(png: &[u8]) -> (Vec<String>, Option<Vec<u8>>) {
    let hits = ocr::find_secrets(png);
    if hits.is_empty() {
        return (Vec::new(), None);
    }
    let mut labels: Vec<String> = Vec::new();
    for h in &hits {
        if !labels.iter().any(|l| l == h.label) {
            labels.push(h.label.to_string());
        }
    }
    (labels, redact_png(png, &hits))
}

fn image_pending(png: Vec<u8>, width: u32, height: u32, auto_hide: bool) -> Pending {
    let (findings, redacted) = if auto_hide {
        scan_image(&png)
    } else {
        (Vec::new(), None)
    };
    Pending {
        clip: Clip {
            id: format!("{:016x}", hash_of(&png)),
            kind: Kind::Image,
            text: None,
            width,
            height,
            bytes: png.len() as u64,
            at: now_ms(),
            favorite: false,
            sensitive: !findings.is_empty(),
            revealed: false,
            findings,
        },
        png: Some(png),
        redacted,
    }
}

fn text_pending(text: String, auto_hide: bool) -> Pending {
    let findings = if auto_hide {
        secrets::labels(&secrets::scan_text(&text))
    } else {
        Vec::new()
    };
    Pending {
        clip: Clip {
            id: format!("{:016x}", hash_of(text.as_bytes())),
            kind: Kind::Text,
            bytes: text.len() as u64,
            text: Some(text),
            width: 0,
            height: 0,
            at: now_ms(),
            favorite: false,
            sensitive: !findings.is_empty(),
            revealed: false,
            findings,
        },
        png: None,
        redacted: None,
    }
}

/// The pasteboard's image as PNG bytes. macOS screenshots copied with ⌃⌘⇧4 are put there as PNG
/// only, which arboard (TIFF only) can't see; other apps may offer TIFF, converted here.
#[cfg(target_os = "macos")]
fn pasteboard_png() -> Option<Vec<u8>> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
        NSPasteboardTypeTIFF,
    };
    use objc2_foundation::NSDictionary;
    autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        // SAFETY: the pasteboard type constants are valid statics; the data is copied out at once.
        if let Some(png) = unsafe { pb.dataForType(NSPasteboardTypePNG) } {
            return Some(png.to_vec());
        }
        let tiff = unsafe { pb.dataForType(NSPasteboardTypeTIFF) }?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(png.to_vec())
    })
}

/// True if the pasteboard content is flagged as secret or temporary by the app that wrote it.
/// Password managers (1Password, Bitwarden, KeePassXC, Apple Passwords…) follow the
/// nspasteboard.org convention; such content must never enter the history.
#[cfg(target_os = "macos")]
fn pasteboard_is_concealed() -> bool {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSPasteboard;
    const MARKERS: [&str; 3] = [
        "org.nspasteboard.ConcealedType",
        "org.nspasteboard.TransientType",
        "com.agilebits.onepassword",
    ];
    autoreleasepool(|_| {
        NSPasteboard::generalPasteboard()
            .types()
            .is_some_and(|types| {
                types
                    .iter()
                    .any(|t| MARKERS.contains(&t.to_string().as_str()))
            })
    })
}

/// Bundle id prefixes of password managers. Many don't mark what they copy, so the app that
/// is in front when the clipboard changes is checked too.
#[cfg(target_os = "macos")]
const PASSWORD_MANAGERS: [&str; 17] = [
    "com.1password.",
    "com.agilebits.",
    "com.bitwarden.",
    "org.keepassxc.",
    "org.keepassx.",
    "com.lastpass.",
    "com.dashlane.",
    "com.sinew.enpass",
    "me.proton.pass",
    "com.nordsec.nordpass",
    "com.markmcguill.strongbox",
    "com.callpod.keepermac",
    "com.apple.keychainaccess",
    "com.apple.passwords",
    "com.apple.systempreferences", // Passwords pane before macOS 15
    "com.siber.roboform",
    "com.mseven.msecure",
];

fn is_password_manager(bundle_id: &str) -> bool {
    let id = bundle_id.to_lowercase();
    PASSWORD_MANAGERS.iter().any(|p| id.starts_with(p))
}

#[cfg(target_os = "macos")]
fn frontmost_is_password_manager() -> bool {
    use objc2_app_kit::NSWorkspace;
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|app| app.bundleIdentifier())
        .is_some_and(|id| is_password_manager(&id.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn frontmost_is_password_manager() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
fn pasteboard_is_concealed() -> bool {
    false
}

/// Reads and analyzes whatever is on the pasteboard. Takes no lock: OCR can take a moment.
fn read_pasteboard(cb: &mut Clipboard, auto_hide: bool) -> Option<Pending> {
    #[cfg(target_os = "macos")]
    if let Some(png) = pasteboard_png() {
        if let Some((width, height)) = png_size(&png) {
            return Some(image_pending(png, width, height, auto_hide));
        }
    }
    if let Ok(img) = cb.get_image() {
        let png = encode_png(&img)?;
        let mut p = image_pending(png, img.width as u32, img.height as u32, auto_hide);
        p.clip.id = format!("{:016x}", hash_of(&img.bytes));
        return Some(p);
    }
    let text = cb.get_text().ok()?;
    if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
        return None;
    }
    Some(text_pending(text, auto_hide))
}

fn encode_png(img: &ImageData) -> Option<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec())?;
    let mut out = Cursor::new(Vec::new());
    buf.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

fn watch(app: AppHandle) {
    thread::spawn(move || {
        let Ok(mut cb) = Clipboard::new() else { return };
        let shared = app.state::<Shared>();
        let mut since_prune = Duration::ZERO;
        let mut manager_was_front = false;
        loop {
            thread::sleep(POLL);
            since_prune += POLL;
            // Also counts the previous tick, so copying a password and switching apps at once is still caught.
            let manager_front = frontmost_is_password_manager();
            let from_manager = manager_was_front || manager_front;
            manager_was_front = manager_front;
            let mut changed = false;
            let (new_content, auto_hide) = {
                let mut s = shared.0.lock().unwrap();
                if since_prune >= PRUNE_EVERY {
                    since_prune = Duration::ZERO;
                    changed |= s.prune();
                }
                let count = change_count();
                let new_content = count != s.last_change;
                if new_content {
                    s.last_change = count;
                }
                (new_content, s.settings.auto_hide)
            };
            if new_content && !from_manager && !pasteboard_is_concealed() {
                // Analysis (OCR) happens before the clip is stored, so it's never visible unredacted.
                if let Some(pending) = read_pasteboard(&mut cb, auto_hide) {
                    shared.0.lock().unwrap().insert(pending);
                    changed = true;
                }
            }
            if changed {
                let _ = app.emit("clips-changed", ());
                refresh_tray(&app);
            }
        }
    });
}

// ───────── Commands ─────────

#[tauri::command]
fn list_clips(state: State<Shared>) -> Vec<Clip> {
    let mut s = state.0.lock().unwrap();
    s.prune();
    s.clips.iter().map(view).collect()
}

/// Returns the clip's image as a base64 PNG.
#[tauri::command]
fn get_image(id: String, state: State<Shared>) -> Result<String, String> {
    let path = {
        let s = state.0.lock().unwrap();
        s.visible_path(&image_clip(&s, &id)?)?
    };
    fs::read(path)
        .map(|b| STANDARD.encode(b))
        .map_err(|e| e.to_string())
}

/// Puts PNG bytes on the pasteboard as-is, so the embedded color profile survives.
#[cfg(target_os = "macos")]
fn set_png(png: &[u8]) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::NSData;
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let data = NSData::with_bytes(png);
    if unsafe { pb.setData_forType(Some(&data), NSPasteboardTypePNG) } {
        Ok(())
    } else {
        Err("couldn't write image to the pasteboard".into())
    }
}

#[cfg(not(target_os = "macos"))]
fn set_png(png: &[u8]) -> Result<(), String> {
    let rgba = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    Clipboard::new()
        .and_then(|mut cb| {
            cb.set_image(ImageData {
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                bytes: rgba.into_raw().into(),
            })
        })
        .map_err(|e| e.to_string())
}

const HIDDEN_ERR: &str = "hidden";

/// Writes a stored clip back to the system clipboard.
fn copy_to_clipboard(s: &mut Store, id: &str) -> Result<(), String> {
    let clip = s
        .clips
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or("clip not found")?;
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    match clip.kind {
        Kind::Text if is_hidden(&clip) => return Err(HIDDEN_ERR.into()),
        Kind::Text => cb
            .set_text(clip.text.unwrap_or_default())
            .map_err(|e| e.to_string())?,
        // Never the full image of a sensitive clip, even while it is revealed in the app.
        Kind::Image => set_png(&fs::read(s.public_path(&clip)?).map_err(|e| e.to_string())?)?,
    }
    // Don't re-capture our own write.
    s.last_change = change_count();
    Ok(())
}

#[tauri::command]
fn copy_clip(id: String, state: State<Shared>) -> Result<(), String> {
    copy_to_clipboard(&mut state.0.lock().unwrap(), &id)
}

#[tauri::command]
fn delete_clip(id: String, app: AppHandle, state: State<Shared>) -> bool {
    let deleted = state.0.lock().unwrap().remove(&id);
    refresh_tray(&app);
    deleted
}

#[tauri::command]
fn set_favorite(id: String, favorite: bool, app: AppHandle, state: State<Shared>) {
    state.0.lock().unwrap().set_favorite(&id, favorite);
    refresh_tray(&app);
}

/// Shows or hides a clip's secrets. Hidden is the default; revealing is the user's call.
#[tauri::command]
fn set_revealed(id: String, revealed: bool, app: AppHandle, state: State<Shared>) {
    state.0.lock().unwrap().set_revealed(&id, revealed);
    refresh_tray(&app);
}

#[tauri::command]
fn clear_clips(app: AppHandle, state: State<Shared>) {
    state.0.lock().unwrap().clear();
    refresh_tray(&app);
}

// ───────── Screenshots ─────────

const PERMISSION_ERR: &str = "screen-recording-permission";

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// True if Screen Recording is granted; otherwise triggers the system prompt (macOS shows it
/// only once per app, so the UI also offers a button that opens System Settings).
fn screen_access() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        if CGPreflightScreenCaptureAccess() {
            return true;
        }
        CGRequestScreenCaptureAccess();
        false
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Whether Screen Recording is granted, without prompting.
#[tauri::command]
fn screen_access_granted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        CGPreflightScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    true
}

#[tauri::command]
fn open_privacy_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn();
}

/// macOS only applies a new Screen Recording grant after the app restarts.
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// Runs a capture started by a global shortcut. The window may be hidden, so failures are
/// reported to it as an event (and the window is brought up for permission problems).
fn capture_from_shortcut(app: &AppHandle, mode: &str) {
    match capture_screenshot(app, mode) {
        Ok(_) => {}
        Err(e) if e == "busy" => {}
        Err(e) => {
            if e == PERMISSION_ERR {
                show_window(app);
            }
            let _ = app.emit("screenshot-error", e);
        }
    }
}

fn screenshots_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .picture_dir()
        .map_err(|e| e.to_string())?
        .join("ClipMac Screenshots");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[derive(Serialize)]
struct ShotResult {
    cancelled: bool,
    id: Option<String>,
    saved: Option<String>,
    /// Secrets were found and hidden.
    hidden: bool,
}

fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(png))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Only one capture at a time (a shortcut can fire while one is already running).
static CAPTURING: AtomicBool = AtomicBool::new(false);

struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::SeqCst);
    }
}

/// Takes a screenshot with the system `screencapture` tool (native resolution, lossless PNG),
/// stores it in the history and puts it on the clipboard.
fn capture_screenshot(app: &AppHandle, mode: &str) -> Result<ShotResult, String> {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return Err("busy".into());
    }
    let _guard = CaptureGuard;
    if !screen_access() {
        return Err(PERMISSION_ERR.into());
    }
    let shared = app.state::<Shared>();
    let shot = shared.0.lock().unwrap().settings.clone();

    // Get our own window out of the picture.
    let win = app.get_webview_window("main");
    let was_visible = win
        .as_ref()
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let was_focused = win
        .as_ref()
        .is_some_and(|w| w.is_focused().unwrap_or(false));
    if was_visible {
        if let Some(w) = &win {
            let _ = w.hide();
        }
        thread::sleep(Duration::from_millis(400));
    }

    let tmp = std::env::temp_dir().join(format!("clipmac-shot-{}.png", now_ms()));
    let mut cmd = Command::new("/usr/sbin/screencapture");
    cmd.args(["-t", "png"]);
    match mode {
        "selection" => {
            cmd.args(["-i", "-s"]);
        }
        "window" => {
            cmd.args(["-i", "-w"]);
            if !shot.shadow {
                cmd.arg("-o");
            }
        }
        _ => {
            if shot.cursor {
                cmd.arg("-C");
            }
        }
    }
    if shot.delay > 0 {
        cmd.arg("-T").arg(shot.delay.to_string());
    }
    let status = cmd.arg(&tmp).status();

    if was_visible {
        if let Some(w) = &win {
            let _ = w.show();
            // Don't steal focus when the capture was started from a shortcut in another app.
            if was_focused {
                let _ = w.set_focus();
            }
        }
    }
    status.map_err(|e| e.to_string())?;

    // Escape during selection exits cleanly without writing a file.
    let Ok(png) = fs::read(&tmp) else {
        return Ok(ShotResult {
            cancelled: true,
            id: None,
            saved: None,
            hidden: false,
        });
    };
    let _ = fs::remove_file(&tmp);
    let (width, height) = png_size(&png).ok_or("screenshot wasn't a valid image")?;

    let pending = image_pending(png, width, height, shot.auto_hide);
    let id = pending.clip.id.clone();
    let hidden = pending.clip.sensitive;

    // Anything leaving the app (clipboard, saved file) is the redacted copy when secrets were found.
    let safe = pending.safe_png().map(<[u8]>::to_vec);
    let saved = match (&safe, shot.save_file) {
        (Some(bytes), true) => {
            let name = format!(
                "Screenshot {}.png",
                chrono::Local::now().format("%Y-%m-%d at %H.%M.%S")
            );
            fs::write(screenshots_dir(app)?.join(name), bytes).map_err(|e| e.to_string())?;
            Some("Pictures/ClipMac Screenshots".to_string())
        }
        _ => None,
    };
    {
        let mut s = shared.0.lock().unwrap();
        if let Some(bytes) = &safe {
            set_png(bytes)?;
        }
        s.last_change = change_count(); // don't re-capture our own write
        s.insert(pending);
    }
    let _ = app.emit("clips-changed", ());
    refresh_tray(app);
    Ok(ShotResult {
        cancelled: false,
        id: Some(id),
        saved,
        hidden,
    })
}

#[tauri::command]
async fn take_screenshot(mode: String, app: AppHandle) -> Result<ShotResult, String> {
    tauri::async_runtime::spawn_blocking(move || capture_screenshot(&app, &mode))
        .await
        .map_err(|e| e.to_string())?
}

/// Saves an edited image (base64 PNG) either over `replace_id` or as a new clip.
/// Returns the id of the saved clip.
#[tauri::command]
fn save_edited(
    png: String,
    replace_id: Option<String>,
    favorite: bool,
    app: AppHandle,
    state: State<Shared>,
) -> Result<String, String> {
    let bytes = STANDARD.decode(png).map_err(|e| e.to_string())?;
    let (width, height) = png_size(&bytes).ok_or("not a valid image")?;
    let auto_hide = state.0.lock().unwrap().settings.auto_hide;
    // Re-scan the edited result (outside the lock): the edit may have added or removed secrets.
    let mut pending = image_pending(bytes, width, height, auto_hide);
    pending.clip.favorite = favorite;

    let id = {
        let mut s = state.0.lock().unwrap();
        let existing =
            replace_id.filter(|id| s.clips.iter().any(|c| &c.id == id && c.kind == Kind::Image));
        match existing {
            Some(id) => {
                let png = pending.png.take().unwrap_or_default();
                fs::write(s.image_path(&id), &png).map_err(|e| e.to_string())?;
                s.write_redacted(&id, pending.redacted.as_deref());
                if let Some(c) = s.clips.iter_mut().find(|c| c.id == id) {
                    c.width = width;
                    c.height = height;
                    c.bytes = png.len() as u64;
                    c.favorite |= favorite;
                    c.sensitive = pending.clip.sensitive;
                    c.revealed = false;
                    c.findings = pending.clip.findings.clone();
                }
                s.save();
                drop_staged(&id);
                id
            }
            None => {
                let id = pending.clip.id.clone();
                s.insert(pending);
                id
            }
        }
    };
    let _ = app.emit("clips-changed", ());
    refresh_tray(&app);
    Ok(id)
}

/// Puts an edited image (base64 PNG) on the clipboard without saving it. The editor may have
/// been showing a revealed image, so the result is scanned again and only a redacted copy
/// reaches the pasteboard. Returns whether secrets were hidden.
#[tauri::command]
fn copy_png(png: String, state: State<Shared>) -> Result<bool, String> {
    let bytes = STANDARD.decode(png).map_err(|e| e.to_string())?;
    let auto_hide = state.0.lock().unwrap().settings.auto_hide;
    // OCR runs outside the lock.
    let (findings, redacted) = if auto_hide {
        scan_image(&bytes)
    } else {
        (Vec::new(), None)
    };
    let hidden = !findings.is_empty();
    let out = if hidden {
        // Fail closed: if the redacted copy couldn't be made, copy nothing.
        redacted.ok_or("couldn't hide the secrets in this image")?
    } else {
        bytes
    };
    let mut s = state.0.lock().unwrap();
    set_png(&out)?;
    s.last_change = change_count(); // don't re-capture our own write
    Ok(hidden)
}

// ───────── Using images as files ─────────

/// A friendly, date-based file name, e.g. "Image 2026-10-16 at 14.03.22.png".
fn clip_file_name(c: &Clip, ext: &str) -> String {
    let when = chrono::DateTime::from_timestamp_millis(c.at as i64)
        .unwrap_or_default()
        .with_timezone(&chrono::Local);
    format!("Image {}.{ext}", when.format("%Y-%m-%d at %H.%M.%S"))
}

fn image_clip(s: &Store, id: &str) -> Result<Clip, String> {
    s.clips
        .iter()
        .find(|c| c.id == id && c.kind == Kind::Image)
        .cloned()
        .ok_or_else(|| "image not found".into())
}

/// Copies the clip's image to a per-clip temp folder under its friendly name, so other apps
/// receive a real file with a sensible name.
fn staged_file(s: &Store, id: &str) -> Result<PathBuf, String> {
    let clip = image_clip(s, id)?;
    let source = s.public_path(&clip)?; // always the redacted copy for a sensitive clip
    drop_staged(id); // never leave an earlier (possibly unredacted) copy behind
    let dir = std::env::temp_dir().join("clipmac-files").join(id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(clip_file_name(&clip, "png"));
    fs::copy(source, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Flattens transparency onto white and encodes a high-quality JPEG.
fn to_jpeg(png: &[u8]) -> Result<Vec<u8>, String> {
    let rgba = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as u32;
        let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Starts a native drag so the image can be dropped into Finder, Mail, Slack, Figma, a browser…
#[tauri::command]
fn start_drag(
    id: String,
    window: tauri::WebviewWindow,
    state: State<Shared>,
) -> Result<(), String> {
    let (path, icon) = {
        let s = state.0.lock().unwrap();
        let path = staged_file(&s, &id)?;
        let thumb = image::load_from_memory(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            .thumbnail(160, 160);
        let mut icon = Cursor::new(Vec::new());
        thumb
            .write_to(&mut icon, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        (path, icon.into_inner())
    };
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let _ = drag::start_drag(
                &target,
                drag::DragItem::Files(vec![path]),
                drag::Image::Raw(icon),
                |_, _| {},
                drag::Options::default(),
            );
        })
        .map_err(|e| e.to_string())
}

/// Puts the image on the clipboard as a *file*, so Cmd+V in Finder, Mail or Slack pastes the file.
#[tauri::command]
fn copy_as_file(id: String, state: State<Shared>) -> Result<(), String> {
    let mut s = state.0.lock().unwrap();
    let path = staged_file(&s, &id)?;
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let ok = Command::new("osascript")
        .args([
            "-e",
            &format!("set the clipboard to (POSIX file \"{escaped}\")"),
        ])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        return Err("couldn't copy the file".into());
    }
    s.last_change = change_count(); // don't re-capture our own write
    Ok(())
}

/// Writes the image to ~/Pictures/ClipMac Screenshots as PNG or JPEG and reveals it in Finder.
#[tauri::command]
fn export_clip(
    id: String,
    format: String,
    app: AppHandle,
    state: State<Shared>,
) -> Result<String, String> {
    let (clip, png) = {
        let s = state.0.lock().unwrap();
        let clip = image_clip(&s, &id)?;
        let bytes = fs::read(s.visible_path(&clip)?).map_err(|e| e.to_string())?; // redacted while hidden
        (clip, bytes)
    };
    let jpeg = format == "jpg";
    let bytes = if jpeg { to_jpeg(&png)? } else { png };
    let dir = screenshots_dir(&app)?;
    let ext = if jpeg { "jpg" } else { "png" };
    let base = clip_file_name(&clip, ext);
    let mut path = dir.join(&base);
    let mut n = 2;
    while path.exists() {
        path = dir.join(base.replace(&format!(".{ext}"), &format!(" {n}.{ext}")));
        n += 1;
    }
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let _ = Command::new("open").arg("-R").arg(&path).spawn();
    Ok("Pictures/ClipMac Screenshots".into())
}

#[tauri::command]
fn get_settings(state: State<Shared>) -> Settings {
    state.0.lock().unwrap().settings.clone()
}

#[tauri::command]
fn set_settings(mut settings: Settings, state: State<Shared>) {
    let mut s = state.0.lock().unwrap();
    settings.shortcuts = s.settings.shortcuts.clone(); // only `set_shortcuts` changes these
    s.settings = settings;
    s.save_settings();
}

// ───────── Code formatting ─────────

fn rustfmt_path() -> PathBuf {
    // GUI apps don't inherit the shell's PATH, so look in the usual places first.
    let home = std::env::var_os("HOME").map(PathBuf::from);
    [
        home.map(|h| h.join(".cargo/bin/rustfmt")),
        Some("/opt/homebrew/bin/rustfmt".into()),
        Some("/usr/local/bin/rustfmt".into()),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.exists())
    .unwrap_or_else(|| PathBuf::from("rustfmt"))
}

fn run_rustfmt(source: &str) -> Result<String, String> {
    use std::io::Write;
    let mut child = Command::new(rustfmt_path())
        .args(["--edition", "2021"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "rustfmt isn’t installed".to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(source.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Formats Rust. Snippets are often bare statements, which rustfmt rejects, so on failure it
/// retries inside a function and strips the wrapper again.
fn format_rust_source(text: &str) -> Result<String, String> {
    if let Ok(formatted) = run_rustfmt(text) {
        return Ok(formatted.trim_end().to_string());
    }
    let wrapped = run_rustfmt(&format!("fn __clip() {{\n{text}\n}}\n"))?;
    let lines: Vec<&str> = wrapped.trim_end().lines().collect();
    if lines.len() < 2 {
        return Err("couldn’t format".into());
    }
    let body = &lines[1..lines.len() - 1];
    Ok(body
        .iter()
        .map(|l| l.strip_prefix("    ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[tauri::command]
async fn format_rust(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || format_rust_source(&text))
        .await
        .map_err(|e| e.to_string())?
}

/// Puts arbitrary text (e.g. a formatted snippet) on the clipboard.
#[tauri::command]
fn copy_text(text: String, state: State<Shared>) -> Result<(), String> {
    let mut s = state.0.lock().unwrap();
    Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .map_err(|e| e.to_string())?;
    s.last_change = change_count(); // don't re-capture our own write
    Ok(())
}

// ───────── Global shortcuts ─────────

/// Registers the given shortcuts (replacing any registered before).
fn register_shortcuts(app: &AppHandle, sc: &Shortcuts) -> Result<(), String> {
    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;
    let mut seen = Vec::new();
    for (mode, accel) in [
        ("selection", &sc.selection),
        ("window", &sc.window),
        ("screen", &sc.screen),
    ] {
        let Some(accel) = accel else { continue };
        let shortcut: Shortcut = accel
            .parse()
            .map_err(|_| format!("“{accel}” isn’t a valid shortcut"))?;
        if seen.contains(&shortcut) {
            return Err("That shortcut is already used for another capture".into());
        }
        seen.push(shortcut);
        let mode = mode.to_string();
        gs.on_shortcut(shortcut, move |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                let (app, mode) = (app.clone(), mode.clone());
                tauri::async_runtime::spawn_blocking(move || {
                    capture_from_shortcut(&app, &mode);
                });
            }
        })
        .map_err(|_| "That shortcut is already in use by macOS or another app".to_string())?;
    }
    Ok(())
}

/// Saves and registers new shortcuts. On failure the previous ones stay active.
#[tauri::command]
fn set_shortcuts(shortcuts: Shortcuts, app: AppHandle, state: State<Shared>) -> Result<(), String> {
    if let Err(e) = register_shortcuts(&app, &shortcuts) {
        let previous = state.0.lock().unwrap().settings.shortcuts.clone();
        let _ = register_shortcuts(&app, &previous);
        return Err(e);
    }
    let mut s = state.0.lock().unwrap();
    s.settings.shortcuts = shortcuts;
    s.save_settings();
    Ok(())
}

/// Pauses the global shortcuts while the user is recording a new one.
#[tauri::command]
fn suspend_shortcuts(app: AppHandle) {
    let _ = app.global_shortcut().unregister_all();
}

#[tauri::command]
fn resume_shortcuts(app: AppHandle, state: State<Shared>) {
    let saved = state.0.lock().unwrap().settings.shortcuts.clone();
    let _ = register_shortcuts(&app, &saved);
}

#[tauri::command]
fn open_screenshots_folder(app: AppHandle) -> Result<(), String> {
    Command::new("open")
        .arg(screenshots_dir(&app)?)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ───────── Menu bar ─────────

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        format!("{}…", flat.chars().take(max).collect::<String>().trim_end())
    } else {
        flat
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let shared = app.state::<Shared>();
    let (clips, deletable): (Vec<Clip>, bool) = {
        let mut s = shared.0.lock().unwrap();
        s.prune();
        (
            s.clips.iter().take(MENU_CLIPS).map(view).collect(),
            s.clips.iter().any(|c| !c.favorite),
        )
    };

    let menu = Menu::new(app)?;
    if clips.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "none",
            "Nothing copied yet",
            false,
            None::<&str>,
        )?)?;
    } else {
        menu.append(&MenuItem::with_id(
            app,
            "hdr",
            "Recent",
            false,
            None::<&str>,
        )?)?;
        for c in &clips {
            let label = match c.kind {
                Kind::Text => one_line(c.text.as_deref().unwrap_or(""), 42),
                Kind::Image => format!("Image · {} × {}", c.width, c.height),
            };
            let hidden = is_hidden(c);
            let label = if c.favorite {
                format!("★ {label}")
            } else {
                label
            };
            let label = if hidden {
                format!("🔒 {label}")
            } else {
                label
            };
            // Hidden text can't be pasted from the menu; reveal it in the app first.
            let enabled = !(hidden && c.kind == Kind::Text);
            menu.append(&MenuItem::with_id(
                app,
                format!("clip:{}", c.id),
                label,
                enabled,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "shot:selection",
        "Capture Selection",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "shot:window",
        "Capture Window",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "shot:screen",
        "Capture Full Screen",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "open",
        "Open ClipMac",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "clear",
        "Clear All",
        deletable,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit ClipMac",
        true,
        Some("Cmd+Q"),
    )?)?;
    Ok(menu)
}

/// Rebuilds the menu-bar menu from the current history.
fn refresh_tray(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (handle.tray_by_id(TRAY_ID), build_menu(&handle)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true) // monochrome; macOS tints it for light/dark menu bars
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "open" => show_window(app),
                "quit" => app.exit(0),
                // Destructive: open the app and let it ask for confirmation.
                "clear" => {
                    show_window(app);
                    let _ = app.emit("request-clear", ());
                }
                _ => {
                    if let Some(mode) = id.strip_prefix("shot:") {
                        let (app, mode) = (app.clone(), mode.to_string());
                        tauri::async_runtime::spawn_blocking(move || {
                            capture_from_shortcut(&app, &mode);
                        });
                    } else if let Some(clip_id) = id.strip_prefix("clip:") {
                        let _ = copy_to_clipboard(
                            &mut app.state::<Shared>().0.lock().unwrap(),
                            clip_id,
                        );
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// Sets the Dock icon at runtime. Bundled builds get it from the .app, but `tauri dev` has no bundle.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../icons/icon-mac.png"));
    if let Some(icon) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: called on the main thread with a valid, retained NSImage.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&icon)) };
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            app.manage(Shared(Mutex::new(Store::load(dir))));
            let saved = app
                .state::<Shared>()
                .0
                .lock()
                .unwrap()
                .settings
                .shortcuts
                .clone();
            let _ = register_shortcuts(app.handle(), &saved); // a taken shortcut shouldn't stop the app
            #[cfg(target_os = "macos")]
            set_dock_icon();
            setup_tray(app.handle())?;
            watch(app.handle().clone());
            Ok(())
        })
        // Closing the window hides it; the app keeps running in the menu bar.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_clips,
            get_image,
            copy_clip,
            delete_clip,
            set_favorite,
            clear_clips,
            take_screenshot,
            screen_access_granted,
            open_privacy_settings,
            restart_app,
            get_settings,
            set_settings,
            format_rust,
            copy_text,
            set_shortcuts,
            suspend_shortcuts,
            resume_shortcuts,
            set_revealed,
            open_screenshots_folder,
            save_edited,
            copy_png,
            start_drag,
            copy_as_file,
            export_clip
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Clicking the Dock icon brings the window back.
            if let RunEvent::Reopen { .. } = event {
                show_window(app);
            }
        });
}

#[cfg(all(test, target_os = "macos"))]
mod pipeline_tests {
    use super::*;

    fn solid_png(rgb: [u8; 3]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(8, 6);
        for p in img.pixels_mut() {
            *p = image::Rgba([rgb[0], rgb[1], rgb[2], 255]);
        }
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// Revealing a clip is for the app window only: what other apps can paste stays redacted.
    #[test]
    fn revealed_sensitive_image_never_reaches_the_pasteboard_unredacted() {
        let _lock = PASTEBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("clipmac-test-{}", now_ms()));
        let mut store = Store::load(dir.clone());
        let (full, redacted) = (solid_png([200, 30, 30]), solid_png([24, 24, 26]));

        let mut pending = image_pending(full.clone(), 8, 6, false);
        pending.clip.sensitive = true;
        pending.redacted = Some(redacted.clone());
        let id = pending.clip.id.clone();
        store.insert(pending);
        store.set_revealed(&id, true);
        assert!(store.clips[0].revealed);

        // The app window may show the full image…
        assert_eq!(
            fs::read(store.visible_path(&store.clips[0]).unwrap()).unwrap(),
            full
        );
        // …but the clipboard and staged files only get the redacted one.
        copy_to_clipboard(&mut store, &id).unwrap();
        assert_eq!(pasteboard_png().as_deref(), Some(&redacted[..]));
        let staged = staged_file(&store, &id).unwrap();
        assert_eq!(fs::read(&staged).unwrap(), redacted);

        drop_staged(&id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recognizes_password_manager_apps() {
        for id in [
            "com.apple.Passwords",
            "com.apple.keychainaccess",
            "com.1password.1password",
            "com.bitwarden.desktop",
        ] {
            assert!(is_password_manager(id), "{id}");
        }
        for id in [
            "com.apple.Safari",
            "com.apple.TextEdit",
            "com.google.Chrome",
        ] {
            assert!(!is_password_manager(id), "{id}");
        }
    }

    /// The system pasteboard is shared by the whole process; tests that use it take turns.
    static PASTEBOARD: Mutex<()> = Mutex::new(());

    #[test]
    fn skips_concealed_pasteboard_content() {
        let _lock = PASTEBOARD.lock().unwrap_or_else(|e| e.into_inner());
        use objc2_app_kit::NSPasteboard;
        use objc2_foundation::{NSArray, NSString};
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let marker = NSString::from_str("org.nspasteboard.ConcealedType");
        unsafe {
            pb.declareTypes_owner(&NSArray::from_slice(&[&*marker]), None);
            pb.setString_forType(&NSString::from_str("hunter2"), &marker);
        }
        assert!(pasteboard_is_concealed());
        pb.clearContents();
        set_png(&[0u8; 4]).unwrap();
        assert!(!pasteboard_is_concealed());
    }

    /// Screenshots copied with ⌃⌘⇧4 are PNG-only on the pasteboard; TIFF-only content must work too.
    #[test]
    fn reads_png_and_tiff_from_pasteboard() {
        let _lock = PASTEBOARD.lock().unwrap_or_else(|e| e.into_inner());
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeTIFF};
        use objc2_foundation::NSData;
        let mut img = image::RgbaImage::new(8, 6);
        for p in img.pixels_mut() {
            *p = image::Rgba([200, 30, 30, 255]);
        }
        let mut png = Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();

        set_png(&png).unwrap();
        assert_eq!(pasteboard_png().as_deref(), Some(&png[..]));

        let mut tiff = Cursor::new(Vec::new());
        img.write_to(&mut tiff, image::ImageFormat::Tiff).unwrap();
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let data = NSData::with_bytes(&tiff.into_inner());
        assert!(unsafe { pb.setData_forType(Some(&data), NSPasteboardTypeTIFF) });
        let got = pasteboard_png().expect("TIFF converted to PNG");
        assert_eq!(png_size(&got), Some((8, 6)));
    }

    /// End-to-end: OCR a real image, find secrets, write the redacted copy.
    /// Runs only when SECRETS_TEST_PNG (input) and SECRETS_TEST_OUT (output) are set.
    #[test]
    fn redacts_secrets_in_screenshot() {
        let (Ok(input), Ok(output)) = (
            std::env::var("SECRETS_TEST_PNG"),
            std::env::var("SECRETS_TEST_OUT"),
        ) else {
            return;
        };
        let png = fs::read(input).unwrap();
        let hits = ocr::find_secrets(&png);
        for h in &hits {
            println!("hit {:<20} rect {:.3?}", h.label, h.rect);
        }
        let (labels, redacted) = scan_image(&png);
        println!("labels: {labels:?}");
        fs::write(output, redacted.expect("a redacted copy")).unwrap();
        assert!(!labels.is_empty());
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    /// The frontend sends `event.code` names; make sure the plugin parses them.
    #[test]
    fn parses_what_the_frontend_records() {
        for accel in [
            "Control+Alt+KeyS",
            "Command+Shift+Digit4",
            "Alt+F5",
            "Control+Shift+Space",
            "Command+Alt+ArrowUp",
            "F13",
        ] {
            assert!(accel.parse::<Shortcut>().is_ok(), "should parse: {accel}");
        }
        assert!("Control+Alt+NotAKey".parse::<Shortcut>().is_err());
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn formats_bare_statements_and_whole_items() {
        if run_rustfmt("fn a(){}").is_err() {
            return; // rustfmt not installed here
        }
        // The snippet from the user's screenshot: statements with no enclosing function.
        let snippet = "format!(\"Hello\");                // => \"Hello\"\nlet people   =  \"Rustaceans\";\nformat!(\"Hello {people}!\");";
        let out = format_rust_source(snippet).unwrap();
        assert!(out.contains("let people = \"Rustaceans\";"), "{out}");
        assert!(!out.contains("__clip"), "{out}");
        assert!(
            !out.lines().any(|l| l.starts_with("    ")),
            "wrapper indent leaked: {out}"
        );

        let item = format_rust_source("fn main(){let x=1;println!(\"{x}\");}").unwrap();
        assert!(item.contains("    let x = 1;"), "{item}");
        assert!(format_rust_source("let = = ;").is_err());
    }
}
