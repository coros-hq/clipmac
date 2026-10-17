// Screenshot editor: background, padding, corners, shadow, zoom/pan, aspect ratio.
// Drawing is a pure function of (image, options, scale), so the on-screen preview and the
// exported PNG come from the same code path.

const GRADIENTS = [
  { id: "aurora", name: "Aurora", angle: 135, stops: ["#667eea", "#764ba2"] },
  { id: "sunset", name: "Sunset", angle: 135, stops: ["#ff7e5f", "#feb47b"] },
  { id: "ocean", name: "Ocean", angle: 135, stops: ["#2193b0", "#6dd5ed"] },
  { id: "mint", name: "Mint", angle: 135, stops: ["#11998e", "#38ef7d"] },
  { id: "candy", name: "Candy", angle: 135, stops: ["#f093fb", "#f5576c"] },
  { id: "dusk", name: "Dusk", angle: 135, stops: ["#4e54c8", "#8f94fb"] },
  { id: "midnight", name: "Midnight", angle: 135, stops: ["#0f2027", "#2c5364"] },
  { id: "peach", name: "Peach", angle: 135, stops: ["#ffecd2", "#fcb69f"] },
];
const SOLIDS = ["#ffffff", "#f2f2f7", "#1c1c1e", "#000000"];
const ASPECTS = { auto: null, "1:1": 1, "4:3": 4 / 3, "16:9": 16 / 9 };
const DEFAULTS = { bg: { type: "gradient", id: "aurora" }, padding: 8, radius: 4, shadow: 40, aspect: "auto" };
const STORE_KEY = "editStyle";
const MAX_SIDE = 16000; // keep exports within canvas limits

const loadStyle = () => {
  try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(STORE_KEY) ?? "{}") }; } catch { return { ...DEFAULTS }; }
};
const saveStyle = (o) => {
  try { localStorage.setItem(STORE_KEY, JSON.stringify({ bg: o.bg, padding: o.padding, radius: o.radius, shadow: o.shadow, aspect: o.aspect })); } catch { /* storage unavailable */ }
};

// ───────── Drawing ─────────

/** Canvas size (in source-image pixels) for the given options. */
function layout(img, o) {
  const w = img.naturalWidth, h = img.naturalHeight;
  const p = (o.padding / 100) * Math.max(w, h);
  let cw = w + 2 * p, ch = h + 2 * p;
  const ratio = ASPECTS[o.aspect];
  if (ratio) { if (cw / ch < ratio) cw = ch * ratio; else ch = cw / ratio; }
  return { w, h, cw, ch };
}

function paintBackground(ctx, bg, W, H) {
  if (bg.type === "solid") { ctx.fillStyle = bg.color; ctx.fillRect(0, 0, W, H); return; }
  if (bg.type !== "gradient") return; // "none": leave transparent
  const g = GRADIENTS.find((x) => x.id === bg.id) ?? GRADIENTS[0];
  const a = (g.angle * Math.PI) / 180;
  const dx = Math.sin(a), dy = -Math.cos(a);
  const half = (Math.abs(W * dx) + Math.abs(H * dy)) / 2; // CSS-style gradient line length
  const grad = ctx.createLinearGradient(W / 2 - dx * half, H / 2 - dy * half, W / 2 + dx * half, H / 2 + dy * half);
  g.stops.forEach((c, i) => grad.addColorStop(i / (g.stops.length - 1), c));
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, W, H);
}

const roundRect = (ctx, x, y, w, h, r) => { ctx.beginPath(); ctx.roundRect(x, y, w, h, r); };

/** Renders into `canvas` at `k` pixels per source pixel. */
function draw(canvas, img, o, k) {
  const L = layout(img, o);
  const W = Math.max(1, Math.round(L.cw * k)), H = Math.max(1, Math.round(L.ch * k));
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, W, H);
  paintBackground(ctx, o.bg, W, H);

  const fw = L.w * k, fh = L.h * k;                 // the screenshot's frame
  const fx = (W - fw) / 2, fy = (H - fh) / 2;
  const r = Math.min(((o.radius / 100) * 0.1) * Math.min(L.w, L.h) * k, fw / 2, fh / 2);

  if (o.shadow > 0) {
    const s = o.shadow / 100;
    const blur = (0.02 + 0.07 * s) * Math.min(fw, fh);
    ctx.save();
    ctx.shadowColor = `rgba(0,0,0,${0.18 + 0.4 * s})`;
    ctx.shadowBlur = blur;
    ctx.shadowOffsetY = blur * 0.35;
    ctx.fillStyle = "#000";
    roundRect(ctx, fx + 0.5, fy + 0.5, fw - 1, fh - 1, r);
    ctx.fill();
    ctx.restore();
  }

  // Zoom crops into the image inside a fixed frame; pan moves the visible region.
  const z = o.zoom ?? 1;
  const dw = fw * z, dh = fh * z;
  const dx = fx - (dw - fw) / 2 + (o.panX ?? 0) * ((dw - fw) / 2);
  const dy = fy - (dh - fh) / 2 + (o.panY ?? 0) * ((dh - fh) / 2);
  ctx.save();
  roundRect(ctx, fx, fy, fw, fh, r);
  ctx.clip();
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, dx, dy, dw, dh);
  ctx.restore();
}

const toBlob = (canvas) => new Promise((res, rej) => canvas.toBlob((b) => (b ? res(b) : rej(new Error("export failed"))), "image/png"));
const blobToBase64 = (blob) =>
  new Promise((res, rej) => {
    const fr = new FileReader();
    fr.onload = () => res(String(fr.result).split(",")[1]);
    fr.onerror = () => rej(fr.error);
    fr.readAsDataURL(blob);
  });

// ───────── UI ─────────

export function createEditor({ root, invoke, imageSrc, h, svg, toast, onSaved }) {
  let session = null; // { clip, img, o, keep, busy, refs }

  const css = (g) => `linear-gradient(135deg, ${g.stops.join(", ")})`;

  async function open(clip) {
    const src = await imageSrc(clip.id);
    if (!src) { toast("Couldn’t load the image"); return; }
    const img = new Image();
    img.src = src;
    try { await img.decode(); } catch { toast("Couldn’t load the image"); return; }
    session = { clip, img, o: { ...loadStyle(), zoom: 1, panX: 0, panY: 0 }, keep: true, busy: false, refs: {} };
    build();
    root.hidden = false;
    document.body.classList.add("editing");
    fit();
  }

  function close() {
    session = null;
    document.body.classList.remove("editing");
    root.hidden = true;
    root.replaceChildren();
  }

  // Drawing at the on-screen size (×DPR for sharpness).
  function fit() {
    if (!session) return;
    const { img, o, refs } = session;
    const L = layout(img, o);
    const availW = Math.max(100, refs.stage.clientWidth - 64);
    const availH = Math.max(100, refs.stage.clientHeight - 64);
    const k = Math.min(availW / L.cw, availH / L.ch, 1);
    session.k = k;
    draw(refs.canvas, img, o, k * (window.devicePixelRatio || 1));
    refs.canvas.style.width = `${L.cw * k}px`;
    refs.canvas.style.height = `${L.ch * k}px`;
    refs.canvas.parentElement.classList.toggle("checker", o.bg.type === "none");
    refs.canvas.classList.toggle("pannable", o.zoom > 1);
  }

  async function exportBlob() {
    const { img, o } = session;
    const L = layout(img, o);
    const canvas = document.createElement("canvas");
    draw(canvas, img, o, Math.min(1, MAX_SIDE / Math.max(L.cw, L.ch)));
    return toBlob(canvas);
  }

  async function run(label, job) {
    if (session.busy) return;
    session.busy = true;
    root.classList.add("busy");
    try { await job(); } catch (e) { toast(`${label} failed`); console.error(e); } finally {
      root.classList.remove("busy");
      if (session) session.busy = false;
    }
  }

  const saveAs = (mode) => run("Save", async () => {
    const { clip, keep } = session;
    const png = await blobToBase64(await exportBlob());
    const id = await invoke("save_edited", { png, replaceId: mode === "overwrite" ? clip.id : null, favorite: keep });
    saveStyle(session.o);
    close();
    await onSaved(id, mode);
  });

  const copyOut = () => run("Copy", async () => {
    const hidden = await invoke("copy_png", { png: await blobToBase64(await exportBlob()) });
    saveStyle(session.o);
    toast(hidden ? "Edited image copied · secrets hidden" : "Edited image copied");
  });

  // ── controls ──
  function slider(label, key, min, max, fmt, scale = 1) {
    const val = h("span", { class: "val" });
    const input = h("input", { type: "range", class: "slider", min, max, step: 1 });
    const sync = () => {
      const v = session.o[key] * scale;
      input.value = v;
      input.style.setProperty("--v", `${((v - min) / (max - min)) * 100}%`);
      val.textContent = fmt(v);
    };
    input.addEventListener("input", () => {
      session.o[key] = Number(input.value) / scale;
      if (key === "zoom") { session.o.panX = Math.max(-1, Math.min(1, session.o.panX)); session.o.panY = Math.max(-1, Math.min(1, session.o.panY)); if (session.o.zoom === 1) { session.o.panX = 0; session.o.panY = 0; } }
      sync();
      fit();
    });
    session.refs.syncs.push(sync);
    sync();
    return h("div", { class: "field" }, h("div", { class: "field-head" }, h("span", {}, label), val), input);
  }

  function build() {
    const refs = (session.refs = { syncs: [] });
    const { o } = session;
    const syncAll = () => refs.syncs.forEach((f) => f());

    // Backgrounds
    const bgItems = [];
    const bgBtn = (title, style, isOn, pick, extra = "") => {
      const b = h("button", { class: `bg-btn ${extra}`, title, "aria-label": title, style, onclick: () => { pick(); syncAll(); fit(); } });
      refs.syncs.push(() => b.classList.toggle("on", isOn()));
      bgItems.push(b);
      return b;
    };
    GRADIENTS.forEach((g) => bgBtn(g.name, `background:${css(g)}`, () => o.bg.type === "gradient" && o.bg.id === g.id, () => (o.bg = { type: "gradient", id: g.id })));
    SOLIDS.forEach((c) => bgBtn(c, `background:${c}`, () => o.bg.type === "solid" && o.bg.color === c, () => (o.bg = { type: "solid", color: c })));
    bgBtn("No background", "", () => o.bg.type === "none", () => (o.bg = { type: "none" }), "none");
    const picker = h("input", { type: "color", class: "bg-btn custom", title: "Custom color", "aria-label": "Custom color" });
    picker.value = o.bg.type === "solid" ? o.bg.color : "#ff6b6b";
    picker.addEventListener("input", () => { o.bg = { type: "solid", color: picker.value }; syncAll(); fit(); });
    refs.syncs.push(() => picker.classList.toggle("on", o.bg.type === "solid" && !SOLIDS.includes(o.bg.color)));

    // Aspect ratio
    const seg = h("div", { class: "seg" }, Object.keys(ASPECTS).map((a) => {
      const b = h("button", { onclick: () => { o.aspect = a; syncAll(); fit(); } }, a === "auto" ? "Auto" : a);
      refs.syncs.push(() => b.classList.toggle("on", o.aspect === a));
      return b;
    }));

    const keep = h("input", { type: "checkbox", class: "switch" });
    keep.checked = session.keep;
    keep.addEventListener("change", () => (session.keep = keep.checked));

    const reset = () => { Object.assign(o, DEFAULTS, { bg: { ...DEFAULTS.bg }, zoom: 1, panX: 0, panY: 0 }); syncAll(); fit(); };

    refs.canvas = h("canvas", { class: "edit-canvas" });
    refs.stage = h("div", { class: "stage" }, h("div", { class: "frame" }, refs.canvas));

    // Drag to pan while zoomed in.
    let drag = null;
    refs.canvas.addEventListener("pointerdown", (e) => {
      if (o.zoom <= 1) return;
      drag = { x: e.clientX, y: e.clientY, px: o.panX, py: o.panY };
      refs.canvas.setPointerCapture(e.pointerId);
      refs.canvas.classList.add("dragging");
    });
    refs.canvas.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const L = layout(session.img, o);
      const limX = ((o.zoom - 1) * L.w * session.k) / 2, limY = ((o.zoom - 1) * L.h * session.k) / 2;
      o.panX = Math.max(-1, Math.min(1, drag.px + (e.clientX - drag.x) / limX));
      o.panY = Math.max(-1, Math.min(1, drag.py + (e.clientY - drag.y) / limY));
      fit();
    });
    const end = () => { drag = null; refs.canvas.classList.remove("dragging"); };
    refs.canvas.addEventListener("pointerup", end);
    refs.canvas.addEventListener("pointercancel", end);

    new ResizeObserver(fit).observe(refs.stage);

    root.replaceChildren(
      h("div", { class: "editor-bar", "data-tauri-drag-region": "" },
        h("button", { class: "btn", onclick: close }, "Cancel"),
        h("div", { class: "editor-title" }, "Edit Image"),
        h("div", { class: "bar-actions" },
          h("button", { class: "btn", onclick: copyOut, title: "Copy the edited image without saving" }, "Copy"),
          h("button", { class: "btn", onclick: () => saveAs("overwrite"), title: "Replace the original image" }, "Overwrite"),
          h("button", { class: "btn primary-btn", onclick: () => saveAs("copy"), title: "Save as a new clip" }, "Save Copy"))),
      h("div", { class: "editor-body" },
        refs.stage,
        h("aside", { class: "inspector" },
          h("div", { class: "section-title" }, "Background"),
          h("div", { class: "bg-grid" }, ...bgItems, picker),
          slider("Padding", "padding", 0, 30, (v) => `${v}%`),
          slider("Corners", "radius", 0, 100, (v) => `${v}`),
          slider("Shadow", "shadow", 0, 100, (v) => `${v}`),
          slider("Zoom", "zoom", 100, 300, (v) => `${v}%`, 100),
          h("div", { class: "hint" }, "Zoom in, then drag the image to pan."),
          h("div", { class: "section-title" }, "Aspect Ratio"),
          seg,
          h("div", { class: "inspector-foot" },
            h("label", { class: "opt" }, h("div", {}, h("div", {}, "Keep forever"), h("div", { class: "hint" }, "Saves as a Favorite so it won’t expire")), keep),
            h("button", { class: "link", onclick: reset }, "Reset")))));
    syncAll();
  }

  window.addEventListener("keydown", (e) => {
    if (session && e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
  }, true);

  return { open, isOpen: () => session !== null };
}
