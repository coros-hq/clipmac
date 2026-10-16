// Code detection, syntax highlighting and formatting for text clips.
// highlight.js (vendor/highlight.min.js) provides the `hljs` global; Prettier is loaded on demand.

const MAX_CHARS = 200_000; // beyond this, show plain text instead of highlighting

const SUBSET = [
  "rust", "javascript", "typescript", "json", "python", "swift", "bash", "css", "scss", "less", "xml",
  "sql", "go", "java", "cpp", "c", "csharp", "yaml", "markdown", "php", "ruby", "kotlin",
];

const LABELS = {
  rust: "Rust", javascript: "JavaScript", typescript: "TypeScript", json: "JSON", python: "Python", swift: "Swift",
  bash: "Shell", shell: "Shell", css: "CSS", scss: "SCSS", less: "Less", xml: "HTML / XML", sql: "SQL", go: "Go",
  java: "Java", cpp: "C++", c: "C", csharp: "C#", yaml: "YAML", markdown: "Markdown", php: "PHP", ruby: "Ruby",
  kotlin: "Kotlin", ini: "Config",
};
export const langLabel = (lang) => LABELS[lang] ?? lang;

const STRONG_START = /^\s*(?:const|let|var|function|class|import|export|fn|pub|use|impl|struct|enum|trait|def|func|package|SELECT|INSERT|UPDATE|CREATE|#include|#!\/|<\?php|<!DOCTYPE|<html|@import|@media|from\s+\S+\s+import)\b/im;
const SHELL_LINE = /^\s*(?:\$\s+)?(?:cargo|rustup|npm|npx|yarn|pnpm|node|git|brew|sudo|cd|ls|cat|echo|export|curl|wget|docker|kubectl|python3?|pip3?|make|mkdir|rm|cp|mv|chmod|ssh|scp|tar|swift|xcodebuild|go|open|killall|defaults)\b/;

/** Is this text code? Returns `{ lang }` (a highlight.js language id) or null.
 *  highlight.js alone is too eager on short text (it will call prose "Rust"), so structural
 *  signals have to agree with it. */
export function detectCode(text) {
  const hljs = globalThis.hljs;
  const t = text.trim();
  if (!hljs || !t || t.length > MAX_CHARS) return null;

  if (/^[\[{]/.test(t) && /[\]}]$/.test(t)) {
    try { JSON.parse(t); return { lang: "json" }; } catch { /* not JSON; keep looking */ }
  }

  // Well-formed markup: a root tag with its closing tag (or a doctype).
  if (/^<!DOCTYPE/i.test(t) || /^<([a-z][\w-]*)[\s>/][\s\S]*<\/\1>\s*$/i.test(t)) return { lang: "xml" };

  const lines = t.split("\n").filter((l) => l.trim());
  if (/^#!\/.*\b(?:ba|z)?sh\b/.test(t)) return { lang: "bash" };
  if (lines.every((l) => SHELL_LINE.test(l) || /^\s*(?:#|&&|\||\\)/.test(l) || /^\s+-{1,2}\w/.test(l))) return { lang: "bash" };
  if (lines.length >= 2 && lines.every((l) => /^\s*(?:export\s+)?[A-Z][A-Z0-9_]*\s*=/.test(l) || /^\s*#/.test(l))) return { lang: "ini" };

  let score = 0;
  if (/[;{}]\s*$/m.test(t)) score++;                                      // statements / blocks
  if (/\{[\s\S]*\}/.test(t)) score++;
  if (/=>|->|::|\+\+|&&|\|\||!==?|===?/.test(t)) score++;                 // operators
  if (STRONG_START.test(t)) score += 2;                                   // declarations
  if ((t.match(/^( {2,}|\t)\S/gm) ?? []).length >= 2) score++;            // indentation
  if (/<\/?[a-z][\w-]*[^>]*>/i.test(t)) score++;                          // markup
  if (/\w\(.*\)/.test(t)) score++;                                        // calls

  const guess = hljs.highlightAuto(t, SUBSET);
  if (!guess.language) return null;
  const r = guess.relevance;
  const isCode = r >= 8 || (score >= 3 && r >= 2) || (score >= 2 && r >= 5);
  return isCode ? { lang: guess.language } : null;
}

/** Highlights `text` and returns one HTML string per line. Spans that cross a newline
 *  (block comments, multi-line strings) are closed and reopened so every line stands alone. */
export function highlightLines(text, lang) {
  const html = globalThis.hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
  const lines = [];
  const open = [];
  let current = "";
  let last = 0;
  const tag = /(<span[^>]*>)|(<\/span>)|\n/g;
  let m;
  while ((m = tag.exec(html))) {
    current += html.slice(last, m.index);
    last = tag.lastIndex;
    if (m[1]) { open.push(m[1]); current += m[1]; }
    else if (m[2]) { open.pop(); current += m[2]; }
    else { lines.push(current + "</span>".repeat(open.length)); current = open.join(""); }
  }
  lines.push(current + html.slice(last));
  return lines;
}

// ── Formatting ──
const PARSERS = { javascript: "babel", typescript: "babel-ts", json: "json", css: "css", scss: "scss", less: "less", xml: "html", markdown: "markdown", yaml: "yaml" };
export const canFormat = (lang) => lang === "rust" || lang in PARSERS;

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const el = document.createElement("script");
    el.src = src;
    el.onload = resolve;
    el.onerror = () => reject(new Error(`couldn't load ${src}`));
    document.head.append(el);
  });
}

let prettierReady;
const loadPrettier = () =>
  (prettierReady ??= (async () => {
    await loadScript("vendor/prettier/standalone.js");
    await Promise.all(["babel", "estree", "postcss", "html", "markdown", "yaml"].map((n) => loadScript(`vendor/prettier/${n}.js`)));
  })().catch((e) => { prettierReady = undefined; throw e; }));

/** Formats `text` as `lang`. Throws if it can't (e.g. a syntax error). */
export async function formatCode(text, lang) {
  if (lang === "rust") return globalThis.__TAURI__.core.invoke("format_rust", { text });
  if (lang === "json") {
    try { return JSON.stringify(JSON.parse(text), null, 2); } catch { /* fall through to a clearer error */ }
  }
  await loadPrettier();
  const out = await globalThis.prettier.format(text, {
    parser: PARSERS[lang],
    plugins: Object.values(globalThis.prettierPlugins),
    tabWidth: 2,
    printWidth: 80,
  });
  return out.replace(/\n$/, "");
}
