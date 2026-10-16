import { createEditor } from "./editor.js";
import { canFormat, detectCode, formatCode, highlightLines, langLabel } from "./code.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const TTL = 24 * 60 * 60 * 1000;

// ───────── Icons ─────────
const ICONS = {
  all: '<path d="M3 4.5h10M3 8h10M3 11.5h6"/>',
  text: '<path d="M3.5 4.5v-1h9v1M8 3.5v9M6 12.5h4"/>',
  image: '<rect x="2.5" y="3" width="11" height="10" rx="1.5"/><circle cx="6" cy="6.5" r="1"/><path d="m3 12 3.5-3.5 2.5 2.5 1.5-1.5 3 3"/>',
  star: '<path d="M8 2.2l1.7 3.5 3.8.5-2.8 2.7.7 3.8L8 10.9l-3.4 1.8.7-3.8L2.5 6.2l3.8-.5z"/>',
  adjust: '<path d="M3 5h6M12 5h1M3 11h1M7 11h6"/><circle cx="10.5" cy="5" r="1.5"/><circle cx="5.5" cy="11" r="1.5"/>',
  camera: '<path d="M2.5 5.5h2l1-1.5h5l1 1.5h2a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1z"/><circle cx="8" cy="9" r="2.4"/>',
  selection: '<rect x="2.5" y="3.5" width="11" height="9" rx="1" stroke-dasharray="2 2"/>',
  window: '<rect x="2.5" y="3" width="11" height="10" rx="1.5"/><path d="M2.5 6h11"/>',
  screen: '<rect x="2" y="3" width="12" height="8" rx="1.2"/><path d="M6 13.5h4"/>',
  lock: '<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5.5a2.5 2.5 0 0 1 5 0V7"/>',
  unlock: '<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5.5a2.5 2.5 0 0 1 4.8-1"/>',
  shield: '<path d="M8 2l5 1.8v4.2c0 3-2.2 5-5 6-2.8-1-5-3-5-6V3.8z"/><path d="m5.8 8 1.7 1.7 2.9-3.1"/>',
  code: '<path d="m5.5 5-3 3 3 3M10.5 5l3 3-3 3M9 3.5l-2 9"/>',
  sparkle: '<path d="M7 2.5l1.4 3.6 3.6 1.4-3.6 1.4L7 12.5 5.6 8.9 2 7.5l3.6-1.4zM12.5 10.5l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6z"/>',
  trash: '<path d="M2.5 4.5h11M6 4.5V3a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v1.5M4 4.5l.6 8a1 1 0 0 0 1 .9h4.8a1 1 0 0 0 1-.9l.6-8"/>',
};

const svg = (name, size = 14, fill = "none") => {
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [k, v] of Object.entries({
    viewBox: "0 0 16 16", width: size, height: size, fill, stroke: "currentColor",
    "stroke-width": 1.4, "stroke-linecap": "round", "stroke-linejoin": "round",
  })) el.setAttribute(k, v);
  el.innerHTML = ICONS[name]; // static, trusted strings only
  return el;
};

const KINDS = {
  text: { label: "Text", plural: "Text", gradient: "linear-gradient(#8e8e93,#6e6e73)" },
  image: { label: "Image", plural: "Images", gradient: "linear-gradient(#ff8a5c,#ff5a3c)" },
};
const CODE_GRADIENT = "linear-gradient(#a78bfa,#7c5cf0)";
const ORDER = ["text", "image"];

// ───────── State ─────────
const SORTS = {
  newest: { label: "Newest", cmp: (a, b) => b.at - a.at },
  oldest: { label: "Oldest", cmp: (a, b) => a.at - b.at },
  largest: { label: "Largest", cmp: (a, b) => b.bytes - a.bytes },
  smallest: { label: "Smallest", cmp: (a, b) => a.bytes - b.bytes },
};
const loadSort = () => { try { const v = localStorage.getItem("sort"); return v in SORTS ? v : "newest"; } catch { return "newest"; } };
const state = { clips: [], filter: "all", query: "", sort: loadSort(), selectedId: null, formatted: null };
const $ = (s) => document.querySelector(s);

// ───────── Helpers ─────────
const h = (tag, attrs = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k.startsWith("aria-")) el.setAttribute(k, String(v));
    else if (typeof v === "boolean") el.toggleAttribute(k, v); // disabled: false must not set the attribute
    else el.setAttribute(k, v);
  }
  el.append(...kids.flat().filter((k) => k != null));
  return el;
};

const timeAgo = (t) => {
  const d = Date.now() - t;
  if (d < 60e3) return "now";
  if (d < 3600e3) return `${Math.floor(d / 60e3)}m`;
  return `${Math.floor(d / 3600e3)}h`;
};

const expiresIn = (c) => {
  if (c.favorite) return "Never";
  const left = Math.max(0, c.at + TTL - Date.now());
  const hrs = Math.floor(left / 3600e3);
  const mins = Math.floor((left % 3600e3) / 60e3);
  return hrs ? `${hrs}h ${mins}m` : `${mins}m`;
};

const formatBytes = (n) =>
  n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;

/** Language info if this text clip is code, else null. Memoized: lists re-render often. */
const codeCache = new Map();
function codeOf(c) {
  if (c.kind !== "text") return null;
  const key = `${c.id}:${c.revealed}`;
  if (!codeCache.has(key)) {
    if (codeCache.size > 500) codeCache.clear();
    codeCache.set(key, detectCode(c.text));
  }
  return codeCache.get(key);
}
const lineCount = (text) => text.split("\n").length;
const isHidden = (c) => c.sensitive && !c.revealed;

const titleOf = (c) =>
  c.kind === "image" ? `${c.width} × ${c.height}` : (c.text.split("\n").find((l) => l.trim()) ?? "").trim();

const subtitleOf = (c) => {
  const code = codeOf(c);
  if (code) { const n = lineCount(c.text); return `${langLabel(code.lang)} · ${n} ${n === 1 ? "line" : "lines"}`; }
  return subtitleBase(c);
};
const subtitleBase = (c) =>
  c.kind === "image" ? `Image · ${formatBytes(c.bytes)}` : `${c.text.length.toLocaleString()} characters`;

// Images are fetched lazily from the backend and cached as data URLs.
const imageCache = new Map();
const imageSrc = (id) => {
  if (!imageCache.has(id)) {
    imageCache.set(id, invoke("get_image", { id }).then((b64) => `data:image/png;base64,${b64}`).catch(() => null));
  }
  return imageCache.get(id);
};
const imageEl = (id, cls) => {
  const img = h("img", { class: cls, alt: "" });
  imageSrc(id).then((src) => src && (img.src = src));
  return img;
};

const matchesFilter = (c) =>
  state.filter === "all" || (state.filter === "favorites" ? c.favorite : c.kind === state.filter);

/** Lets an image clip be dragged out of the app as a real file (Finder, Mail, Slack, Figma…). */
const dragOut = (el, id) => {
  el.draggable = true;
  el.addEventListener("dragstart", (e) => {
    e.preventDefault(); // replace the web drag with a native file drag
    invoke("start_drag", { id }).catch((err) => { toast("Couldn’t start the drag"); console.error(err); });
  });
  return el;
};

/** Clips matching the current filter + search, grouped (Text, then Images), ordered by the chosen sort. */
const visible = () => {
  const q = state.query.trim().toLowerCase();
  return ORDER.flatMap((k) =>
    state.clips
      .filter((c) => c.kind === k && matchesFilter(c) && (!q || (c.text ?? "").toLowerCase().includes(q)))
      .sort(SORTS[state.sort].cmp),
  );
};

// ───────── Render ─────────
function renderNav() {
  const count = (f) => state.clips.filter((c) => f === "all" || (f === "favorites" ? c.favorite : c.kind === f)).length;
  const item = (f, label) =>
    h("button", { class: `nav-item${state.filter === f ? " active" : ""}`, onclick: () => { state.filter = f; render(); } },
      h("span", { class: "glyph" }, svg(f === "favorites" ? "star" : f, 15)),
      h("span", { class: "name" }, label),
      h("span", { class: "num" }, String(count(f))));

  $("#nav").replaceChildren(
    h("div", { class: "nav-label" }, "Library"),
    item("all", "All Clips"),
    item("favorites", "Favorites"),
    h("div", { class: "nav-label" }, "Types"),
    item("text", "Text"),
    item("image", "Images"),
  );
  const n = state.clips.length;
  $("#count").textContent = `${n} ${n === 1 ? "item" : "items"}`;
}

function badge(c) {
  const el = h("div", { class: "badge" });
  if (c.kind === "image") el.append(imageEl(c.id, "thumb"));
  else if (codeOf(c)) { el.style.background = CODE_GRADIENT; el.append(svg("code", 14)); }
  else { el.style.background = KINDS.text.gradient; el.append(svg("text", 14)); }
  return el;
}

function renderList() {
  const items = visible();
  if (!items.some((c) => c.id === state.selectedId)) state.selectedId = items[0]?.id ?? null;

  const list = $("#list");
  if (!items.length) {
    const searching = state.query.trim();
    list.replaceChildren(h("div", { class: "empty" }, h("div", {},
      h("strong", {}, searching ? "No Results" : "Nothing Copied Yet"),
      searching ? `Nothing matches “${state.query}”.` : "Text and images you copy appear here and are kept for 24 hours.")));
    return;
  }

  const nodes = [];
  let last = null;
  for (const c of items) {
    if (c.kind !== last) { nodes.push(h("div", { class: "group-title" }, KINDS[c.kind].plural)); last = c.kind; }
    const row = h("button", {
        class: `row${c.id === state.selectedId ? " selected" : ""}`,
        role: "option",
        "data-id": c.id,
        "aria-selected": c.id === state.selectedId,
        onclick: () => select(c.id),
        ondblclick: () => copyAction(c),
      },
        badge(c),
        h("div", { class: "body" },
          h("div", { class: "title" }, titleOf(c)),
          h("div", { class: "meta" }, subtitleOf(c))),
        h("div", { class: "tail" }, c.sensitive ? h("span", { class: "lock", title: c.revealed ? "Secrets visible" : "Secrets hidden" }, svg(c.revealed ? "unlock" : "lock", 11)) : null, c.favorite ? h("span", { class: "fav" }, svg("star", 11, "currentColor")) : null, h("span", { class: "time" }, timeAgo(c.at)),
          // A span, not a button: rows are already buttons. Favorites are protected, so no delete.
          c.favorite ? null : h("span", {
            class: "row-del", role: "button", title: "Delete", "aria-label": "Delete",
            onclick: (e) => { e.stopPropagation(); remove(c); },
            ondblclick: (e) => e.stopPropagation(),
          }, svg("trash", 13))));
    nodes.push(c.kind === "image" ? dragOut(row, c.id) : row);
  }
  list.replaceChildren(...nodes);
}

/** A syntax-highlighted code block with line numbers. */
function codeBlock(text, lang) {
  const block = h("div", { class: "code-lines" });
  for (const html of highlightLines(text, lang)) {
    const line = h("div", { class: "cl" });
    line.innerHTML = html; // highlight.js escapes the source text; only its own <span>s are markup
    block.append(line);
  }
  return h("div", { class: "canvas code" }, block);
}

function renderPreview() {
  const c = state.clips.find((x) => x.id === state.selectedId);
  const pane = $("#preview");
  if (!c) {
    pane.replaceChildren(h("div", { class: "empty" }, h("div", {}, h("strong", {}, "No Selection"), "Copy something, then select it to preview.")));
    return;
  }
  if (state.formatted && state.formatted.id !== c.id) state.formatted = null;

  const code = codeOf(c);
  const formatted = state.formatted?.id === c.id ? state.formatted.text : null;
  const shown = formatted ?? c.text;

  const canvas = c.kind === "image"
    ? h("div", { class: "canvas image" }, dragOut(imageEl(c.id, "full"), c.id))
    : code ? codeBlock(shown, code.lang)
    : h("div", { class: "canvas text" }, c.text);

  const detail = (k, v) => [h("dt", {}, k), h("dd", {}, v)];
  const secretBar = !c.sensitive ? null : h("div", { class: `secret-bar${c.revealed ? " shown" : ""}` },
    svg(c.revealed ? "unlock" : "lock", 15),
    h("div", { class: "secret-text" },
      h("strong", {}, c.revealed ? "Sensitive content is visible" : "Sensitive content hidden"),
      h("span", {}, c.findings.join(", "))),
    h("button", { class: "btn", onclick: () => setRevealed(c, !c.revealed) }, c.revealed ? "Hide" : "Show"));

  const kindLabel = code ? `Code · ${langLabel(code.lang)}` : KINDS[c.kind].label;
  const canFormatNow = code && canFormat(code.lang) && !isHidden(c); // hidden text is masked; nothing real to format

  // Optional pieces are collected, then filtered: passing null to replaceChildren() would print "null".
  const parts = [
    h("div", { class: "preview-top", "data-tauri-drag-region": "" },
      h("span", { class: "kind" }, kindLabel),
      h("div", { class: "actions" },
        canFormatNow ? h("button", { class: `icon-btn${formatted ? " on" : ""}`, title: formatted ? "Show original" : "Format code", "aria-pressed": String(!!formatted), onclick: () => toggleFormat(c, code.lang) }, svg("sparkle", 15)) : null,
        c.kind === "image" ? h("button", { class: "icon-btn", title: "Edit (⌘E)", onclick: () => editor.open(c) }, svg("adjust", 15)) : null,
        h("button", { class: `icon-btn fav-btn${c.favorite ? " on" : ""}`, title: c.favorite ? "Remove from Favorites" : "Add to Favorites", onclick: () => toggleFavorite(c) }, svg("star", 15, c.favorite ? "currentColor" : "none")),
        h("button", { class: "icon-btn", title: c.favorite ? "Unfavorite to delete" : "Delete", disabled: c.favorite, onclick: () => remove(c) }, svg("trash", 15)))),
    secretBar,
    canvas,
    h("dl", { class: "details" },
      ...detail("Copied", new Date(c.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })),
      ...detail("Expires in", expiresIn(c)),
      ...(code ? detail("Language", langLabel(code.lang)) : []),
      ...(code ? detail("Lines", String(lineCount(shown))) : []),
      ...(c.kind === "image" ? detail("Dimensions", `${c.width} × ${c.height}`) : []),
      ...detail("Size", formatBytes(c.bytes))),
    c.kind === "image" ? h("div", { class: "file-actions" },
      h("button", { class: "btn", title: "Paste it as a file in Finder, Mail, Slack…", onclick: () => copyAsFile(c) }, "Copy as File"),
      h("button", { class: "btn", title: "Save a PNG to Pictures", onclick: () => exportClip(c, "png") }, "Export PNG"),
      h("button", { class: "btn", title: "Save a JPEG to Pictures", onclick: () => exportClip(c, "jpg") }, "Export JPEG")) : null,
    c.kind === "image" ? h("div", { class: "file-hint" }, "Drag the image into any app to drop it as a file.") : null,
    h("button", { class: "primary", onclick: () => copyAction(c) }, formatted ? "Copy Formatted" : "Copy to Clipboard", h("kbd", {}, "⏎")),
  ].filter(Boolean);
  pane.replaceChildren(...parts);
}

const render = () => { renderNav(); renderList(); renderPreview(); };

// ───────── Data + actions ─────────
async function refresh() {
  state.clips = await invoke("list_clips");
  render();
}

function select(id) { state.selectedId = id; renderList(); renderPreview(); }

async function copy(c) {
  try {
    await invoke("copy_clip", { id: c.id });
    toast(c.sensitive && !c.revealed ? "Copied · secrets hidden" : "Copied to Clipboard");
  } catch (e) {
    if (String(e) === "hidden") toast("Show the hidden content first to copy it", 2200);
    else { toast("Couldn’t copy"); console.error(e); }
  }
}

/** Toggles the formatted view of a code clip. The stored clip is never changed. */
async function toggleFormat(c, lang) {
  if (state.formatted?.id === c.id) { state.formatted = null; renderPreview(); return; }
  try {
    state.formatted = { id: c.id, text: await formatCode(c.text, lang) };
    renderPreview();
  } catch (e) {
    toast("Couldn’t format — check the syntax", 2200);
    console.error(e);
  }
}

/** Copies the clip, or its formatted version while the formatted view is on. */
async function copyAction(c) {
  if (state.formatted?.id !== c.id) return copy(c);
  try { await invoke("copy_text", { text: state.formatted.text }); toast("Copied formatted code"); }
  catch (e) { toast("Couldn’t copy"); console.error(e); }
}

async function setRevealed(c, revealed) {
  imageCache.delete(c.id); // the image shown for this clip changes
  await invoke("set_revealed", { id: c.id, revealed });
  await refresh();
}

async function copyAsFile(c) {
  try { await invoke("copy_as_file", { id: c.id }); toast("Copied as a file · paste it anywhere", 2000); }
  catch (e) { toast("Couldn’t copy the file"); console.error(e); }
}

async function exportClip(c, format) {
  try {
    const where = await invoke("export_clip", { id: c.id, format });
    toast(`Exported to ${where}`, 2200);
  } catch (e) { toast("Export failed"); console.error(e); }
}

async function toggleFavorite(c) {
  c.favorite = !c.favorite;
  render();
  await invoke("set_favorite", { id: c.id, favorite: c.favorite });
}

/** In-app confirmation sheet. Resolves true on confirm; Esc / backdrop / Cancel resolve false. */
let dialogOpen = false;
function confirmDialog({ title, message, confirm = "Delete" }) {
  return new Promise((resolve) => {
    dialogOpen = true;
    const previous = document.activeElement;
    const done = (value) => {
      if (!dialogOpen) return;
      dialogOpen = false;
      window.removeEventListener("keydown", onKey, true);
      backdrop.remove();
      previous?.focus?.();
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key !== "Escape" && e.key !== "Enter") return;
      e.preventDefault();
      e.stopPropagation();
      done(e.key === "Enter");
    };
    const confirmBtn = h("button", { class: "dlg-btn danger", onclick: () => done(true) }, confirm);
    const backdrop = h("div", { class: "dialog-backdrop", onmousedown: (e) => { if (e.target === backdrop) done(false); } },
      h("div", { class: "dialog", role: "alertdialog", "aria-modal": "true", "aria-label": title },
        h("div", { class: "dlg-icon" }, svg("trash", 22)),
        h("div", { class: "dlg-title" }, title),
        h("div", { class: "dlg-msg" }, message),
        h("div", { class: "dlg-actions" },
          h("button", { class: "dlg-btn", onclick: () => done(false) }, "Cancel"),
          confirmBtn)));
    document.body.append(backdrop);
    window.addEventListener("keydown", onKey, true);
    confirmBtn.focus();
  });
}

async function remove(c) {
  if (c.favorite) { toast("Unfavorite to delete"); return; }
  const what = c.kind === "image" ? "This image" : "This text";
  if (!(await confirmDialog({ title: "Delete this clip?", message: `${what} will be permanently deleted. This can’t be undone.` }))) return;
  const items = visible();
  const i = items.findIndex((x) => x.id === c.id);
  const next = items[i + 1] ?? items[i - 1];
  state.selectedId = next?.id ?? null;
  state.clips = state.clips.filter((x) => x.id !== c.id);
  imageCache.delete(c.id);
  render();
  await invoke("delete_clip", { id: c.id });
}

let toastTimer;
function toast(msg, ms = 1400) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), ms);
}

// ───────── Editor ─────────
async function onSaved(id, mode) {
  if (mode === "overwrite") imageCache.delete(id); // the file changed under the same id
  state.filter = "all";
  state.query = "";
  $("#search").value = "";
  state.selectedId = id;
  await refresh();
  $(`.row[data-id="${id}"]`)?.scrollIntoView({ block: "nearest" });
  toast(mode === "overwrite" ? "Image overwritten" : "Saved a copy", 1800);
}
const editor = createEditor({ root: $("#editor"), invoke, imageSrc, h, svg, toast, onSaved });

// ───────── Screenshots ─────────
const shot = { settings: { delay: 0, cursor: false, shadow: true, save_file: false, shortcuts: {} }, open: false, busy: false, recording: null };
const MODES = [
  ["selection", "Selection"],
  ["window", "Window"],
  ["screen", "Full Screen"],
];

// ── Shortcut recording ──
const KEY_LABELS = {
  Space: "Space", Enter: "↩", Tab: "⇥", Backspace: "⌫", Delete: "⌦", Escape: "⎋",
  ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→",
  Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Semicolon: ";",
  Quote: "'", Comma: ",", Period: ".", Slash: "/", Backquote: "`",
};
const MOD_SYMBOLS = { Control: "⌃", Alt: "⌥", Shift: "⇧", Command: "⌘" };

/** "Control+Alt+KeyS" → "⌃⌥S" */
function prettyAccel(accel) {
  const parts = accel.split("+");
  const key = parts.pop();
  const label = KEY_LABELS[key] ?? key.replace(/^(Key|Digit|Numpad)/, "");
  return parts.map((m) => MOD_SYMBOLS[m] ?? m).join("") + label;
}

/** Turns a keydown into an accelerator string, or an error message if it isn't usable. */
function accelFromEvent(e) {
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null; // still holding modifiers
  const mods = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Command");
  const isFunctionKey = /^F\d{1,2}$/.test(e.code);
  if (!isFunctionKey && !mods.some((m) => m !== "Shift")) return { error: "Include ⌘, ⌃ or ⌥ in the shortcut" };
  return { accel: [...mods, e.code].join("+") };
}

async function stopRecording({ resume }) {
  shot.recording = null;
  if (resume) await invoke("resume_shortcuts");
  if (shot.open) renderShotPanel();
}

async function startRecording(mode) {
  if (shot.recording) await stopRecording({ resume: false });
  shot.recording = mode;
  await invoke("suspend_shortcuts"); // so an existing shortcut can't fire while recording
  renderShotPanel();
}

async function saveShortcut(mode, accel) {
  const next = { ...shot.settings.shortcuts, [mode]: accel };
  try {
    await invoke("set_shortcuts", { shortcuts: next });
    shot.settings.shortcuts = next;
    shot.recording = null;
    renderShotPanel();
    toast(accel ? `Shortcut set to ${prettyAccel(accel)}` : "Shortcut removed");
  } catch (err) {
    toast(String(err), 3200); // e.g. already in use; the previous shortcuts stay active
    await stopRecording({ resume: true });
  }
}

window.addEventListener("keydown", (e) => {
  if (!shot.recording) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { stopRecording({ resume: true }); return; }
  if ((e.key === "Backspace" || e.key === "Delete") && !(e.metaKey || e.ctrlKey || e.altKey)) { saveShortcut(shot.recording, null); return; }
  const r = accelFromEvent(e);
  if (!r) return;
  if (r.error) { toast(r.error, 2200); return; }
  saveShortcut(shot.recording, r.accel);
}, true);

function setShotPanel(open) {
  if (!open && shot.recording) stopRecording({ resume: true });
  shot.open = open;
  $("#shot-panel").hidden = !open;
  $("#shot-btn").classList.toggle("on", open);
  if (open) renderShotPanel();
}

function updateShot(patch) {
  shot.settings = { ...shot.settings, ...patch };
  invoke("set_settings", { settings: shot.settings });
  renderShotPanel();
}

function renderShotPanel() {
  const { delay, cursor, shadow, save_file } = shot.settings;

  const segmented = (options, value, onpick) =>
    h("div", { class: "seg", role: "group" },
      options.map(([v, label]) => h("button", { class: v === value ? "on" : "", onclick: () => onpick(v) }, label)));

  const toggle = (checked, onchange) => {
    const el = h("input", { type: "checkbox", class: "switch" });
    el.checked = checked;
    el.addEventListener("change", () => onchange(el.checked));
    return el;
  };

  const option = (title, hint, control) =>
    h("div", { class: "opt" },
      h("div", {}, h("div", {}, title), hint ? h("div", { class: "hint" }, hint) : null),
      control);

  $("#shot-panel").replaceChildren(
    h("div", { class: "panel-title" }, "Screenshot"),
    h("div", { class: "modes" },
      MODES.map(([mode, label]) =>
        h("button", { class: "mode", disabled: shot.busy, onclick: () => capture(mode) }, svg(mode, 22), h("span", {}, label)))),
    h("div", { class: "opts" },
      option("Delay", null, segmented([[0, "Off"], [3, "3s"], [5, "5s"], [10, "10s"]], delay, (v) => updateShot({ delay: v }))),
      option("Show cursor", "Full screen only", toggle(cursor, (v) => updateShot({ cursor: v }))),
      option("Window shadow", "Window capture only", toggle(shadow, (v) => updateShot({ shadow: v }))),
      option("Save a copy", "PNG in Pictures › ClipMac Screenshots", toggle(save_file, (v) => updateShot({ save_file: v }))),
      save_file ? h("button", { class: "link", onclick: () => invoke("open_screenshots_folder") }, "Show in Finder") : null),
    h("div", { class: "section-title" }, "Keyboard Shortcuts"),
    h("div", { class: "opts shortcuts" },
      MODES.map(([mode, label]) => {
        const accel = shot.settings.shortcuts?.[mode];
        const recording = shot.recording === mode;
        return h("div", { class: "opt" },
          h("div", {}, label),
          h("div", { class: "key-field" },
            h("button", { class: `key${recording ? " recording" : ""}${accel ? "" : " empty"}`, onclick: () => (recording ? stopRecording({ resume: true }) : startRecording(mode)) },
              recording ? "Press keys…" : accel ? prettyAccel(accel) : "Record"),
            accel && !recording ? h("button", { class: "icon-btn clear-key", title: "Remove shortcut", "aria-label": `Remove ${label} shortcut`, onclick: () => saveShortcut(mode, null) }, "×") : null));
      })),
    h("div", { class: "panel-note" }, "Shortcuts work from any app. Avoid ⌘⇧3/4/5, which macOS uses. Esc cancels recording; ⌫ clears."),
    h("div", { class: "panel-note" }, "Captured at full native resolution as lossless PNG. Esc cancels a selection."));
}

async function capture(mode) {
  if (shot.busy) return;
  shot.busy = true;
  setShotPanel(false);
  try {
    const r = await invoke("take_screenshot", { mode });
    if (r.cancelled) return;
    state.filter = "all";
    state.query = "";
    $("#search").value = "";
    state.selectedId = r.id;
    await refresh();
    $(`.row[data-id="${r.id}"]`)?.scrollIntoView({ block: "nearest" });
    const note = r.hidden ? " · secrets hidden" : "";
    toast((r.saved ? `Screenshot copied · saved to ${r.saved}` : "Screenshot copied to Clipboard") + note, 2600);
  } catch (e) {
    if (String(e) === "busy") return; // a capture is already running
    if (String(e) === "screen-recording-permission") {
      toast("Allow Screen Recording in System Settings, then restart the app", 6000);
    } else {
      toast("Screenshot failed");
      console.error(e);
    }
  } finally {
    shot.busy = false;
  }
}

function renderPrivacy() {
  const on = shot.settings.auto_hide;
  const b = $("#privacy");
  b.classList.toggle("on", on);
  b.title = `Auto-hide secrets: ${on ? "On" : "Off"} (applies to new clips)`;
  b.setAttribute("aria-pressed", String(on));
}
$("#privacy").addEventListener("click", () => {
  updateShot({ auto_hide: !shot.settings.auto_hide });
  renderPrivacy();
  toast(shot.settings.auto_hide ? "Secrets will be hidden automatically" : "Auto-hide is off for new clips", 1800);
});

$("#shot-btn").addEventListener("click", () => setShotPanel(!shot.open));
document.addEventListener("mousedown", (e) => {
  if (shot.open && !e.target.closest("#shot-panel, #shot-btn")) setShotPanel(false);
});
invoke("get_settings").then((s) => { shot.settings = s; renderPrivacy(); });

// ───────── Events ─────────
$("#search").addEventListener("input", (e) => { state.query = e.target.value; renderList(); renderPreview(); });

async function clearAll() {
  const doomed = state.clips.filter((c) => !c.favorite).length;
  if (!doomed) { toast(state.clips.length ? "Favorites are kept" : "Nothing to clear"); return; }
  const kept = state.clips.length - doomed;
  const message = `${doomed} ${doomed === 1 ? "clip" : "clips"} will be permanently deleted${kept ? `; ${kept} ${kept === 1 ? "favorite is" : "favorites are"} kept` : ""}. This can’t be undone.`;
  if (!(await confirmDialog({ title: "Clear clipboard history?", message, confirm: "Clear All" }))) return;
  await invoke("clear_clips");
  await refresh();
  toast(state.clips.length ? "Cleared · favorites kept" : "Cleared all clips");
}
$("#clear").addEventListener("click", clearAll);

const sortSelect = $("#sort");
sortSelect.replaceChildren(...Object.entries(SORTS).map(([k, v]) => h("option", { value: k }, v.label)));
sortSelect.value = state.sort;
sortSelect.addEventListener("change", () => {
  state.sort = sortSelect.value;
  try { localStorage.setItem("sort", state.sort); } catch { /* storage unavailable */ }
  renderList();
});

window.addEventListener("keydown", (e) => {
  if (editor.isOpen() || dialogOpen) return; // the editor / a dialog own the keyboard while open
  const search = $("#search");
  const typing = document.activeElement === search;
  if ((e.metaKey && e.key === "f") || (e.key === "/" && !typing)) { e.preventDefault(); search.focus(); search.select(); return; }
  if (e.key === "Escape" && shot.open) { setShotPanel(false); return; }
  if (e.key === "Escape") { if (search.value) { search.value = ""; state.query = ""; render(); } search.blur(); return; }

  const items = visible();
  const i = items.findIndex((c) => c.id === state.selectedId);
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const n = items[Math.max(0, Math.min(items.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
    if (n) { select(n.id); $(`.row[data-id="${n.id}"]`)?.scrollIntoView({ block: "nearest" }); }
  } else if (e.key === "Enter" && !typing && items[i]) {
    copyAction(items[i]);
  } else if (e.metaKey && e.key === "e" && items[i]?.kind === "image") {
    e.preventDefault();
    editor.open(items[i]);
  } else if (e.metaKey && e.key === "d" && items[i]) {
    e.preventDefault();
    toggleFavorite(items[i]);
  } else if ((e.key === "Backspace" || e.key === "Delete") && e.metaKey && items[i]) {
    remove(items[i]);
  }
});

// Accent-colored selection only while the window is focused.
const setFocused = () => document.body.classList.toggle("focused", document.hasFocus());
window.addEventListener("focus", setFocused);
window.addEventListener("blur", setFocused);
setFocused();

listen("clips-changed", refresh);
listen("request-clear", clearAll); // menu-bar "Clear All" asks here, with confirmation
setInterval(refresh, 30_000); // keeps relative times and the expiry countdown fresh
refresh();
