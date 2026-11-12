// scoping-plan kit — capture mockup frames from the REAL product (fidelity tier "capture").
//
// A plan's capture script (mockups/capture/<surface>.mjs) declares the product build, the
// fixtures and one entry per frame; this harness does the rest:
//   · serves the product's static build from disk by request interception (no server, no deploy),
//   · answers API calls from fixtures, passes only allow-listed static hosts, blocks everything else
//     (nothing is submitted anywhere, no request leaves the machine),
//   · seeds a local session (localStorage / sessionStorage) for signed-in screens,
//   · drives the page to each state (the frame's run()), shoots it at real size × deviceScale,
//   · writes a sidecar <id>.json per frame (page text, console/page errors, unanswered API paths,
//     blocked hosts, fonts, horizontal overflow) that build/review.mjs mockups checks,
//   · regenerates mockups/<surface>.html — the gallery — from the frame list, every figure carrying
//     data-source="capture:<repo>@<sha>".
//
//   import { capture, jwt } from '../../build/capture.mjs';
//   await capture({
//     surface: 'landing-signup',                       // → mockups/landing-signup.html + mockups/assets/landing-signup/
//     source: 'cinefly-frontend@1d319b8d',             // repo@sha the build came from (provenance)
//     intro: 'Rendered from …',                        // gallery intro (HTML)
//     serve: { origin: 'https://site.example', dir: process.env.FE_DIST, spa: false },   // or an array
//     route: '/pricing/',                              // default route for every frame
//     fixtures: { 'https://api.example': { '/v1/me': { id: 7 }, '/v1/cart': (req, url) => ({ … }),
//                                          '/v1/notifications': null } },   // null = empty on purpose
//     ignore: [/Failed to load Stripe\.js/],          // known errors from blocked third parties — say why
//     handlers: [(req, url) => (url.hostname.startsWith('cognito-idp.') ? { body: {} } : undefined)],
//     allow: ['fonts.googleapis.com', 'fonts.gstatic.com'],   // static hosts only — never an API
//     storage: { localStorage: { key: 'value' } },       // applied before any page script runs
//     setup: [helpersFn],                                // page-side functions installed after load
//     frames: [{ id: 's1', title: 'Choose', caption: 'What the frame shows …', section: '§5 step 1',
//                alt: 'Pricing section', route: '/pricing/', session: false,
//                run: async (page, h) => { await h.click('#cta'); } }],
//   });
//
//   node mockups/capture/<surface>.mjs [id …]     capture only these frames (the gallery is always rewritten)
//   LOG=1 node mockups/capture/<surface>.mjs       print every unanswered API path, blocked host and console error
//
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, loadConfig, sleep } from './lib/common.mjs';
import { galleryHtml, frameHtml, caption as captionHtml } from './lib/gallery.mjs';

const TYPES = {
  '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.mjs': 'text/javascript', '.json': 'application/json',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.gif': 'image/gif',
  '.avif': 'image/avif', '.ico': 'image/x-icon', '.woff2': 'font/woff2', '.woff': 'font/woff', '.ttf': 'font/ttf', '.otf': 'font/otf',
  '.mp4': 'video/mp4', '.webm': 'video/webm', '.txt': 'text/plain', '.xml': 'application/xml', '.webmanifest': 'application/manifest+json',
  '.wasm': 'application/wasm', '.symbols': 'text/plain', '.frag': 'text/plain',
};
const CORS = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*', 'access-control-allow-methods': '*' };
const LOG = !!process.env.LOG;

// An unsigned JWT with the given payload: enough for client code that decodes tokens (Amplify,
// auth0-spa, custom hooks) without ever verifying them against a server.
export function jwt(payload, header = { alg: 'RS256', kid: 'fixture' }) {
  const b64 = (o) => Buffer.from(JSON.stringify(o)).toString('base64url');
  return `${b64(header)}.${b64(payload)}.c2lnbmF0dXJl`;
}

// Page-side helpers available in every frame as window.$cap. New UI is composed from the product's
// OWN elements (clone a real field, button or panel so it keeps the component's scoped styles);
// never from new CSS where a product class exists.
function pageHelpers() {
  const q = (s) => (typeof s === 'string' ? document.querySelector(s) : s);
  window.$cap = {
    q,
    // Set an input the way a user would, so React/Vue/Svelte state updates too.
    setValue(sel, value) {
      const el = q(sel); if (!el) throw new Error(`$cap.setValue: no element ${sel}`);
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true }));
      return el;
    },
    text(sel, t) { const el = q(sel); if (!el) throw new Error(`$cap.text: no element ${sel}`); el.textContent = t; return el; },
    clone(sel) { const el = q(sel); if (!el) throw new Error(`$cap.clone: no element ${sel}`); return el.cloneNode(true); },
    hide(sel) { const el = q(sel); if (el) el.style.visibility = 'hidden'; return el; },
    // The innermost element whose text matches (for buttons and labels without stable selectors).
    byText(pattern, scope = 'body *') {
      const re = new RegExp(pattern);
      return [...document.querySelectorAll(scope)].filter((n) => re.test(n.textContent || '')).pop() || null;
    },
    button(pattern) { const re = new RegExp(pattern); return [...document.querySelectorAll('button, a, [role="button"]')].find((b) => re.test(b.textContent || b.getAttribute('aria-label') || '')) || null; },
  };
}

export async function capture(spec) {
  const cfg = await loadConfig();
  for (const k of ['surface', 'source', 'serve', 'frames']) if (!spec[k]) throw new Error(`capture: "${k}" is required`);
  const surface = spec.surface;
  const assetsRel = `mockups/assets/${surface}`;
  const OUT = path.join(ROOT, assetsRel);
  fs.mkdirSync(OUT, { recursive: true });
  const serves = [].concat(spec.serve);
  for (const s of serves) {
    if (!s.dir || !fs.existsSync(s.dir)) throw new Error(`capture: build folder for ${s.origin} not found (${s.dir}) — build the product at the pinned SHA first (see mockups/capture/README.md)`);
  }
  const vp = { width: 1440, height: 900, deviceScale: 2, ...(spec.viewport || {}) };
  const allow = new Set(spec.allow || ['fonts.googleapis.com', 'fonts.gstatic.com']);
  const only = process.argv.slice(2).filter((a) => !a.startsWith('-'));
  const ids = spec.frames.map((f) => f.id);
  for (const id of only) if (!ids.includes(id)) throw new Error(`capture: no frame "${id}" (frames: ${ids.join(', ')})`);
  const dup = ids.find((id, i) => ids.indexOf(id) !== i);
  if (dup) throw new Error(`capture: frame id "${dup}" is used twice`);

  let puppeteer;
  try { ({ default: puppeteer } = await import('puppeteer')); } catch { throw new Error(`puppeteer is not installed for ${ROOT} — (cd "${ROOT}" && npm install)`); }
  const browser = await puppeteer.launch({ headless: 'new' });
  const problems = [];

  async function open(frame) {
    const fvp = { ...vp, ...(frame.viewport || {}) };
    const signedIn = frame.session !== false;
    // Each frame gets its own browser context: no session, storage or cache bleeds between states.
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    await page.setViewport({ width: fvp.width, height: fvp.height, deviceScaleFactor: fvp.deviceScale });
    const log = { unanswered: new Set(), blocked: new Set(), consoleErrors: [], pageErrors: [], failed: new Set() };
    await page.setRequestInterception(true);
    page.on('request', async (req) => {
      const url = new URL(req.url());
      const respond = (r) => req.respond({ status: r.status || 200, contentType: r.contentType || 'application/json', headers: { ...CORS, ...(r.headers || {}) }, body: typeof r.body === 'string' || Buffer.isBuffer(r.body) ? r.body : JSON.stringify(r.body ?? {}) });
      try {
        if (url.protocol === 'data:' || url.protocol === 'blob:') return req.continue();
        const s = serves.find((x) => x.origin === url.origin);
        if (s) {
          let file = path.join(s.dir, decodeURIComponent(url.pathname));
          if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
            const idx = path.join(file, 'index.html');
            file = fs.existsSync(idx) ? idx : (s.spa ? path.join(s.dir, 'index.html') : null);
          }
          if (!file || !fs.existsSync(file)) { if (url.pathname !== '/favicon.ico') log.failed.add(url.pathname); return req.respond({ status: 404, body: '' }); }
          return req.respond({ status: 200, contentType: TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream', headers: CORS, body: fs.readFileSync(file) });   // CORS: fonts served from a second origin (a theme folder) load cross-origin
        }
        if (req.method() === 'OPTIONS') return req.respond({ status: 204, headers: CORS });
        for (const h of spec.handlers || []) {
          const r = await h(req, url);
          if (r) return respond(r);
        }
        const fx = spec.fixtures?.[url.origin];
        if (fx) {
          let v = fx[url.pathname];
          if (typeof v === 'function') v = await v(req, url);
          if (v === undefined) { log.unanswered.add(`${req.method()} ${url.origin}${url.pathname}`); return respond({ body: {} }); }
          if (v === null) return respond({ body: {} });                  // declared empty on purpose
          return respond(v && typeof v === 'object' && 'body' in v && ('status' in v || 'headers' in v) ? v : { body: v });
        }
        if (allow.has(url.hostname)) return req.continue();
        log.blocked.add(url.origin);
        return req.abort();
      } catch (e) {
        log.pageErrors.push(`interception: ${e.message}`);
        try { await req.abort(); } catch { /* already handled */ }
      }
    });
    const ignored = (t) => (spec.ignore || []).some((re) => re.test(t));
    page.on('console', (m) => { if (m.type() === 'error' && !ignored(m.text())) log.consoleErrors.push(m.text().slice(0, 300)); });
    page.on('pageerror', (e) => { const t = String(e.message || e); if (!ignored(t)) log.pageErrors.push(t.slice(0, 300)); });
    if (signedIn && spec.storage) {
      await page.evaluateOnNewDocument((st) => {
        for (const [k, v] of Object.entries(st.localStorage || {})) localStorage.setItem(k, v);
        for (const [k, v] of Object.entries(st.sessionStorage || {})) sessionStorage.setItem(k, v);
      }, spec.storage);
    }
    const origin = frame.origin || serves[0].origin;
    const route = frame.route || spec.route || '/';
    await page.goto(`${origin}${route}`, { waitUntil: 'networkidle0', timeout: 120000 });
    await page.evaluate(() => document.fonts.ready);
    await page.evaluate(pageHelpers);
    for (const fn of spec.setup || []) await page.evaluate(fn);
    await sleep(spec.settle ?? 800);
    return { page, context, log, fvp, route };
  }

  const h = (page) => ({
    wait: (ms) => sleep(ms),
    eval: (fn, ...args) => page.evaluate(fn, ...args),
    // Client-side navigation, as a click inside the app would do it (route guards that redirect a
    // cold page load keep the in-app state this way).
    async inApp(route) {
      await page.evaluate((r) => { history.pushState({}, '', r); dispatchEvent(new PopStateEvent('popstate')); }, route);
      await page.evaluate(() => document.fonts.ready);
      await sleep(spec.settle ?? 1500);
    },
    async click(sel) { await page.click(sel); await sleep(700); },
    async setValue(sel, value) { await page.evaluate((s, v) => window.$cap.setValue(s, v), sel, value); },
    async text(sel, t) { await page.evaluate((s, v) => window.$cap.text(s, v), sel, t); },
  });

  for (const frame of spec.frames) {
    if (only.length && !only.includes(frame.id)) continue;
    const { page, context, log, fvp, route } = await open(frame);
    try {
      if (frame.run) await frame.run(page, h(page));
      await page.evaluate(() => document.fonts.ready);
      await sleep(400);
      const png = path.join(OUT, `${frame.id}.png`);
      await page.screenshot({ path: png, fullPage: !!frame.full });
      const probe = await page.evaluate(() => ({
        text: (document.body.innerText || '').replace(/\s+\n/g, '\n').slice(0, 8000),
        fontsFailed: [...document.fonts].filter((f) => f.status === 'error').map((f) => f.family),
        overflowX: document.documentElement.scrollWidth > window.innerWidth + 1,
      }));
      const side = {
        id: frame.id, source: frame.source || `capture:${spec.source}`, route, viewport: fvp, capturedAt: new Date().toISOString(),
        consoleErrors: log.consoleErrors, pageErrors: log.pageErrors, unanswered: [...log.unanswered], blocked: [...log.blocked],
        missingFiles: [...log.failed], fontsFailed: probe.fontsFailed, overflowX: probe.overflowX, text: probe.text,
      };
      fs.writeFileSync(path.join(OUT, `${frame.id}.json`), JSON.stringify(side, null, 2) + '\n');
      const notes = [
        log.pageErrors.length && `${log.pageErrors.length} page error(s)`,
        log.unanswered.size && `${log.unanswered.size} API call(s) answered with {} (add fixtures: LOG=1)`,
        probe.fontsFailed.length && `fonts failed: ${probe.fontsFailed.join(', ')}`,
        probe.overflowX && 'page scrolls sideways',
      ].filter(Boolean);
      console.log(`  ${notes.length ? '⚠' : '✓'} ${frame.id}${notes.length ? ` — ${notes.join(' · ')}` : ''}`);
      if (LOG) {
        for (const u of log.unanswered) console.log(`      unanswered ${u}`);
        for (const b of log.blocked) console.log(`      blocked ${b}`);
        for (const e of [...log.pageErrors, ...log.consoleErrors]) console.log(`      error ${e}`);
      }
      if (notes.length) problems.push(frame.id);
    } catch (e) {
      throw new Error(`capture ${frame.id}: ${e.message}`);
    } finally {
      await page.close();
      await context.close();
    }
  }
  await browser.close();

  // The gallery is generated from the frame list, so captions and provenance live in one place.
  const shell = spec.shell || (vp.width <= 500 ? 'phone' : 'desktop');
  const frames = spec.frames.map((f) => {
    const fvp = { ...vp, ...(f.viewport || {}) };
    const src = `assets/${surface}/${f.id}.png`;
    if (!fs.existsSync(path.join(ROOT, 'mockups', src))) console.log(`  ⚠ ${f.id}: no capture yet (${src})`);
    return frameHtml({
      id: f.id, source: f.source || `capture:${spec.source}`, shell, width: fvp.width, height: f.full ? null : fvp.height,
      body: `<img src="${src}" alt="${String(f.alt || f.title).replace(/"/g, '&quot;')}">`,
      captionHtml: captionHtml({ id: f.id, title: f.title, text: f.caption, section: f.section }),
    });
  });
  const script = path.relative(ROOT, process.argv[1] || `mockups/capture/${surface}.mjs`);
  const gallery = path.join(ROOT, 'mockups', `${surface}.html`);
  fs.writeFileSync(gallery, galleryHtml({ name: cfg.name, surface, intro: spec.intro || `Rendered from ${spec.source}.`, frames, generatedBy: script }));
  const registered = cfg.mockups.some((m) => m.file === `mockups/${surface}.html`);
  console.log(`✔ ${path.relative(ROOT, gallery)}: ${frames.length} frame(s)${only.length ? ` (captured ${only.join(', ')})` : ''}${registered ? '' : ` — ⚠ not in plan.config.mjs mockups; add { prefix, kind: 'desktop', file: 'mockups/${surface}.html', deviceScale: 2 }`}`);
  if (problems.length) console.log(`  ⚠ frames with capture problems: ${problems.join(', ')} — fix the fixtures or the run() until each is clean (LOG=1 lists them)`);
  console.log('  next: node build/build-plan.mjs --shots  → mockups/shots/, then node build/review.mjs mockups');
}
