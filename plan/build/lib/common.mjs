// scoping-plan kit — shared helpers used by every kit script.
// Installed into <plan>/build/lib/. Change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const KIT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');   // <plan>/build
export const ROOT = path.join(KIT, '..');                                          // <plan>

// ---------------------------------------------------------------- theme
// The kit theme, overridden key by key by the plan's own <plan>/theme.json when it has one (its
// colours, fonts, font stylesheet and logo). Every kit script reads THEME, so one file restyles all.
const mergeTheme = (a, b) => {
  const o = { ...a };
  for (const [k, v] of Object.entries(b)) o[k] = v && typeof v === 'object' && !Array.isArray(v) && a[k] && typeof a[k] === 'object' ? mergeTheme(a[k], v) : v;
  return o;
};
const PLAN_THEME = path.join(ROOT, 'theme.json');
export const THEME = mergeTheme(
  JSON.parse(fs.readFileSync(path.join(KIT, 'theme.json'), 'utf8')),
  fs.existsSync(PLAN_THEME) ? JSON.parse(fs.readFileSync(PLAN_THEME, 'utf8')) : {},
);
export const COLOR = THEME.color;
// The display face for headings, when the theme names one.
export const DISPLAY_FONT = THEME.font.display || THEME.font.sans;

export function rgb(hex) {
  const h = hex.replace('#', '');
  const f = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  return [0, 2, 4].map((i) => parseInt(f.slice(i, i + 2), 16));
}
export const rgba = (token, a) => `rgba(${rgb(COLOR[token] || token).join(', ')}, ${a})`;

// Every hex a document may use: the theme palette plus pure white/black.
export const TOKEN_HEXES = new Set([...Object.values(COLOR), ...Object.values(THEME.deck?.tailwindColors || {}), '#fff', '#ffffff', '#000', '#000000']
  .map((h) => h.toLowerCase()));

// {{c:token}} · {{rgba:token:alpha}} · {{font:sans}} → values. Used for framework.css and templates.
export function applyTheme(text) {
  return text
    .replace(/\{\{c:(\w+)\}\}/g, (m, k) => { if (!COLOR[k]) throw new Error(`theme: unknown colour ${k}`); return COLOR[k]; })
    .replace(/\{\{rgba:(\w+):([\d.]+)\}\}/g, (m, k, a) => { if (!COLOR[k]) throw new Error(`theme: unknown colour ${k}`); return rgba(k, a); })
    .replace(/\{\{font:(\w+)\}\}/g, (m, k) => THEME.font[k]);
}

// ---------------------------------------------------------------- plan config
export async function loadConfig() {
  const p = path.join(ROOT, 'plan.config.mjs');
  if (!fs.existsSync(p)) throw new Error(`no plan.config.mjs in ${ROOT} — run the skill's scripts/plan.mjs init first`);
  const cfg = (await import(pathToFileURL(p).href)).default;
  for (const k of ['name', 'master', 'html']) if (!cfg[k]) throw new Error(`plan.config.mjs: "${k}" is required`);
  return {
    brand: THEME.brand,
    diagramsSrc: 'diagrams/src',
    diagramsOut: 'diagrams/drawio',
    mockups: [],
    generators: {},
    ...cfg,
  };
}

export const read = (rel) => fs.readFileSync(path.join(ROOT, rel), 'utf8');
export const exists = (rel) => fs.existsSync(path.join(ROOT, rel));

// ---------------------------------------------------------------- draw.io CLI
// Resolution order: $DRAWIO_BIN → macOS app → drawio / draw.io on PATH.
export function drawioBin() {
  const candidates = [
    process.env.DRAWIO_BIN,
    '/Applications/draw.io.app/Contents/MacOS/draw.io',
    path.join(process.env.HOME || '', 'Applications/draw.io.app/Contents/MacOS/draw.io'),
  ].filter(Boolean);
  for (const c of candidates) if (fs.existsSync(c)) return c;
  for (const name of ['drawio', 'draw.io']) {
    try { return execFileSync('which', [name], { encoding: 'utf8' }).trim(); } catch { /* next */ }
  }
  throw new Error('draw.io Desktop CLI not found. Install draw.io Desktop (https://github.com/jgraph/drawio-desktop/releases) or set DRAWIO_BIN. On headless Linux run the build under xvfb-run.');
}

// ---------------------------------------------------------------- headless browser
export async function browser() {
  let puppeteer;
  try { ({ default: puppeteer } = await import('puppeteer')); } catch {
    throw new Error(`puppeteer is not installed for ${ROOT} — run: (cd "${ROOT}" && npm install)`);
  }
  return puppeteer.launch({ headless: 'shell', args: ['--allow-file-access-from-files'] });
}
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Link to the vendored fonts from a file at `fromRel` (relative to the plan root).
// The theme's font stylesheet (plan-relative), linked from a page at plan-relative path fromRel.
export function fontsHref(fromRel = '') {
  const depth = fromRel.split('/').length - 1;
  return `${depth ? '../'.repeat(depth) : ''}${THEME.fontsCss || 'build/vendor/fonts.css'}`;
}

// Relative link from one plan file to another (both relative to the plan root).
export function relLink(fromFile, target) {
  const rel = path.relative(path.dirname(path.join(ROOT, fromFile)), path.join(ROOT, target));
  return rel.startsWith('.') ? rel : `./${rel}`;
}

export const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
