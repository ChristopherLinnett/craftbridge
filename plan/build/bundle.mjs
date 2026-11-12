// Shareable bundle: one zip with the rendered plan and the presentation, where every figure,
// font, mockup, script and audio file resolves relatively — the input a publishing tool
// (e.g. a deploy-plan skill) takes. Built by crawling what the entry pages actually reference.
//   node build/bundle.mjs            → <plan>/<slug>-plan.zip  (+ a resolution check of the unzipped copy)
// Never bundled: analysis/ (deployed snapshots can embed credentials), review/, node_modules/,
// PLAN-STATE.json. evidence/ only when plan.config.mjs sets bundle: { include: ['evidence'] }
// (research notes can carry infrastructure detail; the bundle may be published). Links into
// excluded folders are reported, not followed. A plan theme's own asset folder (the folder holding
// theme.json's fontsCss) is bundled whole, so font licences and logo sources travel with the fonts.
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { ROOT, THEME, loadConfig } from './lib/common.mjs';

const cfg = await loadConfig();
const optIn = new Set((cfg.bundle?.include || []).filter((f) => f !== 'analysis'));
const EXCLUDE = new RegExp(`^(analysis|review|node_modules${optIn.has('evidence') ? '' : '|evidence'})(/|$)|^PLAN-STATE\\.json$|\\.zip$`);
const entries = [cfg.html, 'presentation/index.html'].filter((f) => fs.existsSync(path.join(ROOT, f)));
if (!fs.existsSync(path.join(ROOT, cfg.html))) { console.error(`✘ ${cfg.html} not built — run node build/build-plan.mjs`); process.exit(1); }

const files = new Set();
const skipped = new Map();
const missing = new Map();
const queue = [...entries];
const refsOf = (rel, text) => {
  const ext = path.extname(rel);
  let found = [];
  if (ext === '.html') found = [...text.matchAll(/\b(?:src|href)="([^"#?]+)/g)].map((m) => m[1]);
  if (ext === '.css') found = [...text.matchAll(/url\(\s*['"]?([^'")#?]+)/g)].map((m) => m[1]);
  if (ext === '.js') found = [...text.matchAll(/['"]((?:audio|assets)\/[^'"]+\.(?:mp3|m4a|wav|ogg|png|jpe?g|webp|mp4))['"]/g)].map((m) => m[1]);
  // Skip absolute URLs and template expressions inside page scripts (${…}, {{…}}).
  return found.filter((u) => !/^(?:[a-z]+:|\/\/|data:|mailto:)/i.test(u) && !/\$\{|\{\{/.test(u));
};
while (queue.length) {
  const rel = queue.shift();
  if (files.has(rel)) continue;
  const abs = path.join(ROOT, rel);
  if (!fs.existsSync(abs)) continue;
  files.add(rel);
  if (!/\.(html|css|js)$/.test(rel)) continue;
  const text = fs.readFileSync(abs, 'utf8');
  for (const u of refsOf(rel, text)) {
    const target = path.normalize(path.join(path.dirname(rel), decodeURIComponent(u)));
    if (target.startsWith('..')) { missing.set(target, rel); continue; }
    if (EXCLUDE.test(target)) { skipped.set(target, rel); continue; }
    if (!fs.existsSync(path.join(ROOT, target))) { missing.set(target, rel); continue; }
    if (fs.statSync(path.join(ROOT, target)).isDirectory()) { skipped.set(target, rel); continue; }
    queue.push(target);
  }
}
// Narration clips are listed in autopilot-data.js; include the whole audio folder when present.
const audio = path.join(ROOT, 'presentation', 'audio');
if (fs.existsSync(audio)) for (const f of fs.readdirSync(audio)) if (/\.(mp3|m4a|wav|ogg)$/.test(f)) files.add(`presentation/audio/${f}`);

// A plan's own theme folder (fonts.css beside its fonts) travels whole, so font licences, the logo
// and their source notes go wherever the fonts go.
if (THEME.fontsCss && !THEME.fontsCss.startsWith('build/')) {
  const walk = (d) => {
    for (const f of fs.readdirSync(path.join(ROOT, d))) {
      const r = path.join(d, f);
      if (fs.statSync(path.join(ROOT, r)).isDirectory()) walk(r); else if (!/^\.DS_Store$/.test(f)) files.add(r);
    }
  };
  walk(path.dirname(THEME.fontsCss));
}

const slug = cfg.name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
const zipPath = path.join(ROOT, `${slug}-plan.zip`);
fs.rmSync(zipPath, { force: true });
const list = [...files].sort();
execFileSync('zip', ['-q', '-X', zipPath, ...list], { cwd: ROOT });

// Resolution check on the unzipped copy: every local reference from every bundled page must exist.
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'plan-bundle-'));
execFileSync('unzip', ['-q', zipPath, '-d', tmp]);
const broken = [];
for (const rel of list.filter((f) => /\.(html|css)$/.test(f))) {
  for (const u of refsOf(rel, fs.readFileSync(path.join(tmp, rel), 'utf8'))) {
    const target = path.normalize(path.join(path.dirname(rel), decodeURIComponent(u)));
    if (!fs.existsSync(path.join(tmp, target)) && !skipped.has(target) && !missing.has(target)) broken.push(`${rel} → ${u}`);
  }
}
// Rendered check: open every bundled entry page from the unzipped copy the way a reader will, with
// every lazy image forced in, and require that each image decodes, no local request fails and the
// theme font loads. (A static crawl cannot see a broken image or a missing font.)
const rendered = [];
try {
  const { browser, sleep } = await import('./lib/common.mjs');
  const b = await browser();
  try {
    for (const entry of entries) {
      const page = await b.newPage();
      const failed = [];
      page.on('requestfailed', (r) => { if (r.url().startsWith('file:')) failed.push(decodeURIComponent(r.url().replace(`file://${tmp}/`, ''))); });
      await page.setViewport({ width: 1440, height: 1000 });
      await page.goto(`file://${path.join(tmp, entry)}`, { waitUntil: 'load', timeout: 120000 });
      const r = await page.evaluate(async (font) => {
        const imgs = [...document.images];
        imgs.forEach((i) => { i.loading = 'eager'; });
        await Promise.all(imgs.map((i) => Promise.race([i.decode().catch(() => null), new Promise((res) => setTimeout(res, 8000))])));
        await document.fonts.ready;
        return { images: imgs.length, broken: imgs.filter((i) => !i.naturalWidth).map((i) => i.getAttribute('src')), font: document.fonts.check(`15px "${font}"`) };
      }, THEME.font.sans);
      await sleep(300);
      for (const x of r.broken) rendered.push(`${entry}: image does not load — ${x}`);
      for (const f of [...new Set(failed)]) rendered.push(`${entry}: request failed — ${f}`);
      if (!r.font) rendered.push(`${entry}: the theme font did not load`);
      console.log(`  rendered ${entry}: ${r.images} image(s), ${r.broken.length} broken, ${failed.length} failed request(s), font ${r.font ? 'ok' : 'MISSING'}`);
      await page.close();
    }
  } finally { await b.close(); }
} catch (e) {
  if (/puppeteer is not installed/.test(e.message)) console.log('  ⚠ rendered check skipped (puppeteer not installed)'); else throw e;
}
fs.rmSync(tmp, { recursive: true, force: true });

const size = (fs.statSync(zipPath).size / 1024 / 1024).toFixed(1);
console.log(`✔ ${path.basename(zipPath)} · ${list.length} files · ${size} MB`);
console.log(`  entries: plan ${cfg.html}${entries.includes('presentation/index.html') ? ' · presentation presentation/index.html' : ' · (no presentation)'}`);
if (skipped.size) console.log(`  not bundled by rule (links to them will not resolve in the bundle): ${[...new Set([...skipped.keys()].map((k) => k.split('/')[0]))].join(', ')}`);
for (const [t, from] of missing) console.log(`  ⚠ referenced but missing: ${t} (from ${from})`);
for (const x of rendered) console.log(`  ✘ ${x}`);
if (broken.length) broken.forEach((b) => console.log(`  ✘ unresolved in bundle: ${b}`));
if (broken.length || rendered.length) process.exit(1);
