// Visual review: renders what a reader will see, runs rendered-DOM checks, and writes PNGs to
// review/ for the model to LOOK at. Build logs are not a review; the PNGs are.
//
//   node build/review.mjs diagrams [id-filter]  → review/diagrams/<id>.png + REPORT.md
//   node build/review.mjs html                  → review/html/NN-<section>[-k].png + REPORT.md
//   node build/review.mjs mockups               → checks mockups/shots/*.png (blank frames) + REPORT.md
//   node build/review.mjs deck [dir]            → review/deck/NN-<id>.png + REPORT.md (default dir presentation)
//   node build/review.mjs mermaid               → review/mermaid/<id>.png — the markdown twins, rendered + syntax-checked
//   node build/review.mjs anchors [--evidence]  → review/anchors/REPORT.md — every code anchor resolved at its pin, with
//                                                 its excerpt, the identifiers its sentence names, and the moving ground
//   node build/review.mjs all
// Exit code 1 when any check reports an error. Warnings still need a human-grade look.
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, COLOR, THEME, loadConfig, browser, sleep, fontsHref } from './lib/common.mjs';
import { mermaidBlocks, renderMermaid } from './lib/mermaid.mjs';
import { findAnchors, resolveAnchors, anchorsReportMd } from './lib/anchors.mjs';
import { movingGround, movingGroundMd } from './lib/repos.mjs';

const cfg = await loadConfig();
const [what = 'all', arg] = process.argv.slice(2);
const OUT = path.join(ROOT, 'review');
let totalErrors = 0;

function writeReport(dir, title, items, rubric) {
  const lines = [`# ${title}`, '', `Generated ${new Date().toISOString().slice(0, 16).replace('T', ' ')} by build/review.mjs. Automated checks only: open every PNG listed and judge it against the rubric before marking the piece done.`, ''];
  for (const it of items) {
    lines.push(`## ${it.name}`, '', `- PNG: \`${path.relative(ROOT, it.png)}\``);
    if (!it.errors.length && !it.warnings.length) lines.push('- automated checks: clean');
    for (const e of it.errors) lines.push(`- ✘ ${e}`);
    for (const w of it.warnings) lines.push(`- ⚠ ${w}`);
    lines.push('');
  }
  lines.push('## Rubric (judge each PNG; any "no" means redo, not patch)', '', ...rubric.map((r) => `- [ ] ${r}`), '');
  fs.writeFileSync(path.join(dir, 'REPORT.md'), lines.join('\n'));
}
function summarise(kind, items) {
  const e = items.reduce((n, i) => n + i.errors.length, 0);
  const w = items.reduce((n, i) => n + i.warnings.length, 0);
  totalErrors += e;
  for (const it of items) {
    const tag = it.errors.length ? '✘' : it.warnings.length ? '⚠' : '✔';
    console.log(`${tag} ${it.name}${it.errors.length ? ` · ${it.errors.length} error(s)` : ''}${it.warnings.length ? ` · ${it.warnings.length} warning(s)` : ''}`);
    for (const x of it.errors) console.log(`    ✘ ${x}`);
    for (const x of it.warnings) console.log(`    ⚠ ${x}`);
  }
  console.log(`${kind}: ${items.length} item(s), ${e} error(s), ${w} warning(s) → review/${kind}/REPORT.md`);
}

// A real file (not setContent) so file:// fonts and images load.
function stagePage(name, html) {
  const p = path.join(ROOT, 'review', `.${name}.html`);
  fs.writeFileSync(p, html);
  return p;
}
const fontLink = `<link rel="stylesheet" href="${fontsHref('review/.page.html')}">`;
const FONT = THEME.font.sans;

// ---------------------------------------------------------------- diagrams
const DIAGRAM_RUBRIC = [
  'The title states the one question this diagram answers, and the diagram answers it at a glance.',
  'Reading order is obvious (left→right or top→bottom); nothing makes the eye jump back.',
  'Nothing overlaps, clips or crowds: every label sits inside its box with breathing room; spacing follows one rhythm.',
  'No edge passes behind a node, doubles back, or crosses another where a waypoint or reorder would avoid it.',
  'Edges that carry meaning are labelled; labels sit on their edge, not on a node.',
  'Colour means something and a legend says what; only kit kinds are used.',
  'Text is readable at the embedded size (nothing tiny); Part A diagrams use business language, not identifiers.',
  'Where it shows data or a flow, it uses the plan\'s running example; it agrees with the master text and its mermaid twin.',
  'It looks like it belongs with the other diagrams (same title block, margins, density).',
  'Trace test: follow every edge from its source to its target without losing it. An edge you lose (it crosses others, detours around the outside, zigzags, runs through a title) is a defect — re-layout.',
  'Glance test: cover the title, look for five seconds, then say what the diagram shows. If that is not the title\'s statement, the diagram is redone.',
];

function parseDrawio(xml) {
  // Attribute order differs between cells, so read each tag's attributes as a map.
  const cells = [];
  for (const m of xml.matchAll(/<mxCell\b([^>]*?)\/?>/g)) {
    const a = Object.fromEntries([...m[1].matchAll(/([\w:-]+)="([^"]*)"/g)].map((x) => [x[1], x[2]]));
    if (a.vertex !== '1' && a.edge !== '1') continue;
    const style = a.style || '';
    let kind;
    if (a.edge === '1') kind = a.source && a.target ? 'edge' : 'line';
    else if (style.startsWith('text;')) kind = 'text';
    else if (style.includes('absoluteArcSize=1')) kind = 'bar';
    else if (style.includes('arcSize=3;') && style.includes('spacingLeft=14')) kind = 'group';
    else kind = 'node';
    cells.push({ id: a.id, kind, source: a.source, target: a.target, hasLabel: (a.value || '').length > 0 });
  }
  return cells;
}

async function reviewDiagrams(b) {
  const dir = path.join(OUT, 'diagrams');
  fs.mkdirSync(dir, { recursive: true });
  const src = path.join(ROOT, cfg.diagramsOut);
  const ids = fs.readdirSync(src).filter((f) => f.endsWith('.svg') && (!arg || f.includes(arg))).map((f) => f.replace(/\.svg$/, '')).sort();
  const items = [];
  const page = await b.newPage();
  for (const id of ids) {
    const svg = fs.readFileSync(path.join(src, `${id}.svg`), 'utf8').replace(/^<\?xml[^>]*>\s*/, '').replace(/<!DOCTYPE[^>]*>\s*/, '');
    const drawio = path.join(src, `${id}.drawio`);
    const cells = fs.existsSync(drawio) ? parseDrawio(fs.readFileSync(drawio, 'utf8')) : [];
    const w = +(svg.match(/width="([\d.]+)px"/) || [0, 1400])[1];
    const h = +(svg.match(/height="([\d.]+)px"/) || [0, 800])[1];
    await page.setViewport({ width: Math.ceil(w + 48), height: Math.ceil(h + 48), deviceScaleFactor: 1.5 });
    const file = stagePage('diagram', `<!doctype html><html><head>${fontLink}<style>body{margin:0;background:${COLOR.page}}#wrap{display:inline-block;padding:24px;background:${COLOR.page}}svg{display:block}</style></head><body><div id="wrap">${svg}</div></body></html>`);
    await page.goto('file://' + file, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);
    await sleep(150);
    const res = await page.evaluate((cellList, font) => {
      const errors = []; const warnings = [];
      if (!document.fonts.check(`12px "${font}"`)) errors.push(`the ${font} font did not load — this render is not representative`);
      const R = (r) => ({ l: r.left, t: r.top, r: r.right, b: r.bottom, w: r.width, h: r.height });
      const info = {};
      for (const c of cellList) {
        const g = document.querySelector(`g[data-cell-id="${CSS.escape(c.id)}"]`);
        if (!g) continue;
        const shapeEl = g.querySelector(':scope > g > rect, :scope > g > path, :scope > g > ellipse');
        const lab = [...g.querySelectorAll('foreignObject div')].find((d) => d.style.display === 'inline-block');
        info[c.id] = {
          ...c,
          shape: shapeEl ? R(shapeEl.getBoundingClientRect()) : null,
          label: lab && lab.textContent.trim() ? R(lab.getBoundingClientRect()) : null,
          text: lab ? lab.innerText.replace(/\s*\n\s*/g, ' · ').trim().slice(0, 44) : '',
          path: c.kind === 'edge' ? g.querySelector(':scope > g > path') : null,
        };
      }
      const name = (x) => `"${x.text || x.id}"`;
      const vals = Object.values(info);
      // 1. labels that spill out of their box
      for (const x of vals) {
        if (!x.shape || !x.label || x.kind === 'edge' || x.kind === 'line') continue;
        const s = x.shape; const l = x.label; const tol = 2;
        const spill = Math.max(s.l - l.l, l.r - s.r, s.t - l.t, l.b - s.b);
        if (spill > tol) (x.kind === 'text' ? warnings : errors).push(`${x.kind} ${name(x)} label overflows its ${x.kind === 'text' ? 'text area' : 'box'} by ${Math.round(spill)}px`);
      }
      // 2. edges routed through a node that is neither end
      const nodes = vals.filter((x) => x.kind === 'node' && x.shape);
      for (const e of vals.filter((x) => x.kind === 'edge' && x.path)) {
        const len = e.path.getTotalLength();
        const ctm = e.path.getScreenCTM();
        const hit = new Set();
        for (let d = 0; d <= len; d += 3) {
          const p = new DOMPoint(e.path.getPointAtLength(d).x, e.path.getPointAtLength(d).y).matrixTransform(ctm);
          for (const n of nodes) {
            if (n.id === e.source || n.id === e.target) continue;
            const s = n.shape; const m = 4;
            if (p.x > s.l + m && p.x < s.r - m && p.y > s.t + m && p.y < s.b - m) hit.add(n.id);
          }
        }
        for (const n of hit) errors.push(`edge ${e.source}→${e.target} passes through node ${name(info[n])}`);
        // edge line drawn through someone else's text (group titles, notes, node labels)
        const crossed = new Set();
        for (let dd = 0; dd <= len; dd += 3) {
          const p = new DOMPoint(e.path.getPointAtLength(dd).x, e.path.getPointAtLength(dd).y).matrixTransform(ctm);
          for (const x of vals) {
            if (x === e || !x.label || x.kind === 'edge') continue;
            const l = x.label;
            if (p.x > l.l + 1 && p.x < l.r - 1 && p.y > l.t + 1 && p.y < l.b - 1) crossed.add(x.id);
          }
        }
        for (const c of crossed) warnings.push(`edge ${e.source}→${e.target} runs through the text of ${info[c].kind} ${name(info[c])}`);
        // edge label sitting on a node
        if (e.label) for (const n of nodes) {
          if (n.id === e.source || n.id === e.target) continue;
          const a = e.label; const s = n.shape;
          const ov = Math.max(0, Math.min(a.r, s.r) - Math.max(a.l, s.l)) * Math.max(0, Math.min(a.b, s.b) - Math.max(a.t, s.t));
          if (ov > 20) warnings.push(`edge label "${e.text}" sits on node ${name(n)}`);
        }
      }
      // 3. labels colliding with other labels
      const labs = vals.filter((x) => x.label);
      for (let i = 0; i < labs.length; i++) for (let j = i + 1; j < labs.length; j++) {
        const a = labs[i].label; const c = labs[j].label;
        const ov = Math.max(0, Math.min(a.r, c.r) - Math.max(a.l, c.l)) * Math.max(0, Math.min(a.b, c.b) - Math.max(a.t, c.t));
        if (ov > 20) warnings.push(`labels ${name(labs[i])} and ${name(labs[j])} collide`);
      }
      // 4. edge geometry — the shapes a reader's eye loses: detours, runs outside the content, jogs,
      //    zigzags and crossings. Each is fixed by re-layout (reorder, restack, split), not by waypoints.
      const contentNodes = nodes.filter((n) => !/^lg_/.test(n.id));
      const box = contentNodes.reduce((b, n) => ({ l: Math.min(b.l, n.shape.l), t: Math.min(b.t, n.shape.t), r: Math.max(b.r, n.shape.r), b: Math.max(b.b, n.shape.b) }), { l: Infinity, t: Infinity, r: -Infinity, b: -Infinity });
      const groupsR = vals.filter((x) => x.kind === 'group' && x.shape).map((g) => g.shape);
      const outer = groupsR.reduce((b, g) => ({ l: Math.min(b.l, g.l), t: Math.min(b.t, g.t), r: Math.max(b.r, g.r), b: Math.max(b.b, g.b) }), box);
      const polys = [];
      for (const e of vals.filter((x) => x.kind === 'edge' && x.path)) {
        const L = e.path.getTotalLength(); const ctm = e.path.getScreenCTM();
        const pts = [];
        for (let d = 0; d <= L; d += 2) { const p = new DOMPoint(e.path.getPointAtLength(d).x, e.path.getPointAtLength(d).y).matrixTransform(ctm); pts.push([p.x, p.y]); }
        if (pts.length < 2) continue;
        // Direction runs: H/V, with rounded-corner curves (diagonal samples) kept apart as 'X'.
        let runs = [];
        for (let i = 1; i < pts.length; i++) {
          const dx = pts[i][0] - pts[i - 1][0]; const dy = pts[i][1] - pts[i - 1][1];
          const len = Math.hypot(dx, dy); if (len < 0.2) continue;
          const dir = Math.abs(dx) > 0.35 * len && Math.abs(dy) > 0.35 * len ? 'X' : Math.abs(dx) >= Math.abs(dy) ? (dx > 0 ? 'R' : 'L') : (dy > 0 ? 'D' : 'U');
          const last = runs[runs.length - 1];
          if (last && last.dir === dir) { last.len += len; last.end = pts[i]; } else runs.push({ dir, len, start: pts[i - 1], end: pts[i] });
        }
        // The straight stretches a reader follows: runs longer than 20 px (corners and tiny wiggles drop out).
        runs = runs.filter((r) => r.dir !== 'X' && r.len > 20);
        if (!runs.length) continue;
        const axis = (d) => (d === 'L' || d === 'R' ? 'H' : 'V');
        const [x0, y0] = pts[0]; const [x1, y1] = pts[pts.length - 1];
        const lenPx = runs.reduce((n, r) => n + r.len, 0);
        const direct = Math.abs(x1 - x0) + Math.abs(y1 - y0);
        const tag = `edge ${e.source}→${e.target}`;
        if (e.source !== e.target && lenPx > 220 && lenPx > 1.9 * Math.max(direct, 40)) warnings.push(`${tag} takes a detour: ${Math.round(lenPx)}px of line for ${Math.round(direct)}px between its ends — move the two nodes closer or restack, don't route around`);
        // Wrapping the diagram (outside on two sides, or along most of one side) is the margin detour;
        // a short loop just above or below one row is fine.
        const sides = { t: pts.filter(([, y]) => y < outer.t - 6), b: pts.filter(([, y]) => y > outer.b + 6), l: pts.filter(([x]) => x < outer.l - 6), r: pts.filter(([x]) => x > outer.r + 6) };
        const span = (arr, k) => (arr.length ? Math.max(...arr.map((p) => p[k])) - Math.min(...arr.map((p) => p[k])) : 0);
        const out = Object.entries(sides).filter(([, v]) => v.length > 3);
        const long = (sides.t.length && span(sides.t, 0) > 0.6 * (outer.r - outer.l)) || (sides.b.length && span(sides.b, 0) > 0.6 * (outer.r - outer.l))
          || (sides.l.length && span(sides.l, 1) > 0.6 * (outer.b - outer.t)) || (sides.r.length && span(sides.r, 1) > 0.6 * (outer.b - outer.t));
        if (out.length >= 2 || long) warnings.push(`${tag} wraps around the outside of the diagram (${out.map(([k]) => ({ t: 'top', b: 'bottom', l: 'left', r: 'right' })[k]).join(' + ')}) — re-layout so it runs between the nodes`);
        if (runs.length - 1 > 4) warnings.push(`${tag} bends ${runs.length - 1} times — a reader loses it; reorder so the path is direct`);
        // A jog: two straight stretches on the same axis, a few px apart — a line that should be straight.
        for (let i = 1; i < runs.length; i++) {
          const a = runs[i - 1]; const c = runs[i];
          if (axis(a.dir) !== axis(c.dir)) continue;
          const off = axis(a.dir) === 'H' ? Math.abs(c.start[1] - a.end[1]) : Math.abs(c.start[0] - a.end[0]);
          if (off > 1.5 && off < 18) { warnings.push(`${tag} has a ${Math.round(off)}px jog — align its exit and entry (same y or x fraction) so the line runs straight`); break; }
        }
        polys.push({ tag, e, verts: [pts[0], ...runs.flatMap((r) => [r.start, r.end]), pts[pts.length - 1]], ends: [pts[0], pts[pts.length - 1]] });
      }
      const cross = (p, q, r, s2) => {
        const d = (a, b, c) => (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        const d1 = d(r, s2, p); const d2 = d(r, s2, q); const d3 = d(p, q, r); const d4 = d(p, q, s2);
        return ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0));
      };
      const near = (pt, ends) => ends.some((e) => Math.hypot(pt[0] - e[0], pt[1] - e[1]) < 14);
      for (let i = 0; i < polys.length; i++) for (let j = i + 1; j < polys.length; j++) {
        const A = polys[i]; const B = polys[j];
        let hit = null;
        for (let a = 1; a < A.verts.length && !hit; a++) for (let b = 1; b < B.verts.length && !hit; b++) {
          const p = A.verts[a - 1]; const q = A.verts[a]; const r = B.verts[b - 1]; const s2 = B.verts[b];
          if (cross(p, q, r, s2)) {
            // intersection point, ignoring meetings at a shared node
            const den = (p[0] - q[0]) * (r[1] - s2[1]) - (p[1] - q[1]) * (r[0] - s2[0]);
            const t = ((p[0] - r[0]) * (r[1] - s2[1]) - (p[1] - r[1]) * (r[0] - s2[0])) / den;
            const pt = [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])];
            if (!near(pt, [...A.ends, ...B.ends])) hit = pt;
          }
        }
        if (hit) warnings.push(`${A.tag} crosses ${B.tag.replace('edge ', '')} — reorder nodes or lanes so they don't; if the flow really interleaves, it belongs in a sequence diagram`);
      }
      // 5. tidiness — near-misses read as mistakes: nodes a few px out of line, nodes outside every group.
      for (let i = 0; i < contentNodes.length; i++) for (let j = i + 1; j < contentNodes.length; j++) {
        const a = contentNodes[i].shape; const c = contentNodes[j].shape;
        const dy = Math.abs(a.t - c.t); const dx = Math.abs(a.l - c.l);
        const sideBySide = Math.abs(a.h - c.h) < 2 && (a.r < c.l || c.r < a.l) && Math.min(Math.abs(a.r - c.l), Math.abs(c.r - a.l)) < 420;
        const stacked = Math.abs(a.w - c.w) < 2 && (a.b < c.t || c.b < a.t) && Math.min(Math.abs(a.b - c.t), Math.abs(c.b - a.t)) < 300;
        if (sideBySide && dy > 0.9 && dy < 9) warnings.push(`nodes ${name(contentNodes[i])} and ${name(contentNodes[j])} are ${Math.round(dy)}px out of line — put them on the same y`);
        else if (stacked && dx > 0.9 && dx < 9) warnings.push(`nodes ${name(contentNodes[i])} and ${name(contentNodes[j])} are ${Math.round(dx)}px out of column — put them on the same x`);
      }
      if (groupsR.length) {
        for (const n of contentNodes) {
          if (/^(h_|s\d)/.test(n.id)) continue;
          const s = n.shape;
          if (!groupsR.some((g) => s.l >= g.l - 1 && s.r <= g.r + 1 && s.t >= g.t - 1 && s.b <= g.b + 1)) warnings.push(`node ${name(n)} sits outside every group — put it in the group it belongs to (or give it one)`);
        }
      }
      return { errors, warnings };
    }, cells, FONT);
    const png = path.join(dir, `${id}.png`);
    const el = await page.$('#wrap');
    await el.screenshot({ path: png });
    if (!cells.length) res.warnings.push(`no ${id}.drawio next to the SVG — rendered checks skipped`);
    items.push({ name: id, png, ...res });
  }
  await page.close();
  writeReport(dir, 'Diagram review', items, DIAGRAM_RUBRIC);
  summarise('diagrams', items);
}

// ---------------------------------------------------------------- html
const HTML_RUBRIC = [
  'Fonts render in the theme font (a serif fallback means the fonts did not load).',
  'Headings form a clean hierarchy; the sidebar lists every Part and section; no stray markers (@, %%, {{ }}).',
  'Every figure is sized to be read without zooming and sits next to the text that explains it.',
  'Tables fit or scroll deliberately; no empty header bars; code blocks readable.',
  'Part A reads as a business document (no identifiers needed to follow it); Part B/C carry the detail.',
  'Nothing describes how the plan was made: it states intent, decisions and design only.',
];
async function reviewHtml(b) {
  const dir = path.join(OUT, 'html');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const page = await b.newPage();
  await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
  await page.goto('file://' + path.join(ROOT, cfg.html), { waitUntil: 'load' });
  await page.evaluate(() => document.fonts.ready);
  // Figures are lazy-loaded: force every image in, or off-screen ones read as broken and tile blank.
  await page.evaluate(async () => {
    const imgs = [...document.querySelectorAll('img')];
    imgs.forEach((i) => { i.loading = 'eager'; });
    await Promise.all(imgs.map((i) => (i.complete && i.naturalWidth ? null : new Promise((r) => { i.addEventListener('load', r, { once: true }); i.addEventListener('error', r, { once: true }); setTimeout(r, 8000); }))));
  });
  await sleep(500);
  const dom = await page.evaluate((font) => {
    const errors = []; const warnings = [];
    if (!document.fonts.check(`15px "${font}"`)) errors.push(`the ${font} font did not load`);
    document.querySelectorAll('img').forEach((i) => { if (!i.complete || !i.naturalWidth) errors.push(`image failed to load: ${i.getAttribute('src')}`); });
    const text = document.querySelector('main').innerText;
    for (const [re, what] of [[/@pending|@drawio|@view|@end\b|@gen\b/, 'build marker'], [/%%/, 'mermaid comment'], [/\{\{[^}]*\}\}/, 'template placeholder'], [/\bundefined\b|\bNaN\b|\[object Object\]/, 'rendering artefact']]) {
      const m = text.match(re); if (m) errors.push(`${what} visible in the page: "${text.slice(Math.max(0, m.index - 30), m.index + 40).replace(/\s+/g, ' ')}"`);
    }
    document.querySelectorAll('.table-wrap').forEach((t) => {
      if (t.scrollWidth > t.clientWidth + 4) warnings.push(`table scrolls sideways (${t.scrollWidth}px in ${t.clientWidth}px) near "${(t.previousElementSibling?.textContent || '').slice(0, 50)}"`);
    });
    document.querySelectorAll('figure.diagram svg').forEach((s) => { const r = s.getBoundingClientRect(); if (r.width < 100 || r.height < 60) errors.push(`diagram renders at ${Math.round(r.width)}×${Math.round(r.height)}px`); });
    let last = 1;
    document.querySelectorAll('main h1, main h2, main h3, main h4').forEach((h) => { const l = +h.tagName[1]; if (l > last + 1) warnings.push(`heading level jumps h${last}→h${l} at "${h.textContent.replace('#', '').trim().slice(0, 50)}"`); last = l; });
    const seen = new Set();
    document.querySelectorAll('main h2').forEach((h) => { const t = h.textContent.replace('#', '').trim(); if (seen.has(t)) warnings.push(`duplicate section title "${t}"`); seen.add(t); });
    // Section boundaries for the tiles.
    const heads = [...document.querySelectorAll('main h1, main h2')];
    const secs = heads.map((h, i) => ({
      title: h.textContent.replace('#', '').trim(),
      top: h.getBoundingClientRect().top + window.scrollY - 24,
      bottom: (heads[i + 1] ? heads[i + 1].getBoundingClientRect().top + window.scrollY : document.documentElement.scrollHeight) - 8,
    }));
    return { errors, warnings, secs, x: document.querySelector('main').getBoundingClientRect().left };
  }, FONT);
  // Local links must resolve.
  const html = fs.readFileSync(path.join(ROOT, cfg.html), 'utf8');
  const base = path.dirname(path.join(ROOT, cfg.html));
  for (const m of html.matchAll(/\bhref="(?!https?:|#|mailto:|data:)([^"#?]+)/g)) {
    if (!fs.existsSync(path.join(base, decodeURIComponent(m[1])))) dom.errors.push(`broken local link: ${m[1]}`);
  }
  const items = [];
  const slugify = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);
  let n = 0;
  const MAX = 2200;
  for (const s of dom.secs) {
    n += 1;
    const h = s.bottom - s.top;
    // A leftover under 60 px folds into the tile before it: a sliver tile shows nothing, and one
    // under a pixel high cannot be captured at all.
    const parts = Math.max(1, Math.ceil((h - 60) / MAX));
    for (let k = 0; k < parts; k++) {
      const y = s.top + k * MAX;
      const height = k === parts - 1 ? s.bottom - y : MAX;
      if (height < 1) continue;
      const png = path.join(dir, `${String(n).padStart(2, '0')}-${slugify(s.title)}${parts > 1 ? `-${k + 1}` : ''}.png`);
      await page.screenshot({ path: png, clip: { x: dom.x, y, width: 1440 - dom.x, height }, captureBeyondViewport: true });
      items.push({ name: `${s.title}${parts > 1 ? ` (${k + 1}/${parts})` : ''}`, png, errors: [], warnings: [] });
    }
  }
  await page.close();
  items.unshift({ name: `${cfg.html} (page-wide checks)`, png: path.join(dir, '01-*.png'), errors: dom.errors, warnings: dom.warnings });
  writeReport(dir, 'Plan HTML review', items, HTML_RUBRIC);
  summarise('html', items.slice(0, 1));
  console.log(`   ${items.length - 1} section tile(s) in review/html/ — read them in order`);
}

// ---------------------------------------------------------------- mockups
const MOCKUP_RUBRIC = [
  'Provenance: every frame is a capture of the real product at its pin (or its real template / the provider default); a hand rebuild says in the gallery intro why the product could not be built.',
  'The real screen surrounds the feature: real header, real page behind the modal, real neighbouring content, the product font and wordmark (never styled text).',
  'New UI is the product\'s own components (cloned fields, panels, buttons), not new CSS; any value that could not be cloned cites its source (file:line).',
  'The state is coherent: nothing on screen contradicts it (no close button on a non-dismissable step, no leftover error on a success frame, no terms line on a signing-in card).',
  'Running example everywhere: the same names, emails, ids, prices and dates as the master; derived values computed with the product\'s own logic.',
  'Every state in the flow matrix (evidence/flows.md) and every §5 step has a frame; frames are numbered in flow order; each sits next to its step in the master.',
  'Nothing cut off, blank or clipped; no plan or process text inside a frame; captions say what is shown, cite the PD/L/D it embodies and end with "Frame xN · §…".',
  'Side-by-side test: put each new-state frame next to the untouched capture of the same screen; every difference is the feature, never a drift in font, spacing, colour or copy.',
];
const PLACEHOLDER = /\blorem\b|\bipsum\b|\bplaceholder\b|\bJohn Doe\b|\bJane Doe\b|\bfoo\b|\bbar baz\b|example\.com|\bTODO\b|\bTBD\b|\{\{|\}\}|\bundefined\b|\bNaN\b|\[object Object\]/i;
function galleryFrames() {
  const map = new Map();
  for (const m of cfg.mockups) {
    const file = path.join(ROOT, m.file);
    if (!fs.existsSync(file)) continue;
    const html = fs.readFileSync(file, 'utf8');
    for (const f of html.matchAll(/<figure class="frame"([^>]*)>([\s\S]*?)<\/figure>/g)) {
      const id = (f[1].match(/\bid="([^"]+)"/) || [])[1];
      if (!id) continue;
      const source = (f[1].match(/\bdata-source="([^"]*)"/) || [])[1] || null;
      const img = (f[2].match(/<img[^>]*\bsrc="([^"]+)"/) || [])[1] || null;
      const sidecar = img && /\.png$/.test(img) ? path.join(path.dirname(file), img.replace(/\.png$/, '.json')) : null;
      map.set(id, { gallery: m.file, source, sidecar: sidecar && fs.existsSync(sidecar) ? sidecar : null, caption: (f[2].match(/<figcaption>([\s\S]*?)<\/figcaption>/) || [])[1] || '' });
    }
  }
  return map;
}
async function reviewMockups(b) {
  const dir = path.join(OUT, 'mockups');
  fs.mkdirSync(dir, { recursive: true });
  const shots = path.join(ROOT, 'mockups', 'shots');
  const files = fs.existsSync(shots) ? fs.readdirSync(shots).filter((f) => f.endsWith('.png')).sort((a, c) => a.localeCompare(c, undefined, { numeric: true })) : [];
  const frames = galleryFrames();
  const page = await b.newPage();
  const items = [];
  const thumbs = {};
  const sizes = {};
  for (const f of files) {
    const id = f.replace('.png', '');
    const res = { errors: [], warnings: [] };
    await page.goto('file://' + path.join(shots, f));
    const stats = await page.evaluate(async () => {
      const img = document.querySelector('img');
      await img.decode();
      const c = document.createElement('canvas');
      const w = Math.min(400, img.naturalWidth); const h = Math.round(img.naturalHeight * (w / img.naturalWidth));
      c.width = w; c.height = h;
      const ctx = c.getContext('2d'); ctx.drawImage(img, 0, 0, w, h);
      const d = ctx.getImageData(0, 0, w, h).data;
      const counts = new Map();
      for (let i = 0; i < d.length; i += 16) { const k = (d[i] >> 4) * 256 + (d[i + 1] >> 4) * 16 + (d[i + 2] >> 4); counts.set(k, (counts.get(k) || 0) + 1); }
      const top = Math.max(...counts.values());
      // 48×30 grey thumbnail for near-duplicate detection
      const t = document.createElement('canvas'); t.width = 48; t.height = 30;
      const tc = t.getContext('2d'); tc.drawImage(img, 0, 0, 48, 30);
      const td = tc.getImageData(0, 0, 48, 30).data; const g = [];
      for (let i = 0; i < td.length; i += 4) g.push(Math.round(0.3 * td[i] + 0.59 * td[i + 1] + 0.11 * td[i + 2]));
      return { share: top / (d.length / 16), w: img.naturalWidth, h: img.naturalHeight, g };
    });
    thumbs[id] = stats.g; sizes[id] = `${stats.w}×${stats.h}`;
    if (stats.share > 0.97) res.errors.push(`frame looks blank (${Math.round(stats.share * 100)}% one colour)`);
    if (stats.h < 120) res.warnings.push(`frame is only ${stats.h}px tall — did the screen render?`);
    const fr = frames.get(id);
    if (!fr) res.warnings.push('no figure.frame with this id in any registered gallery (a stale shot?)');
    else {
      if (!fr.source) res.warnings.push('the frame has no data-source — record its provenance: capture:<repo>@<sha> · template:<path> · provider:<what> · rebuild:<ui-spec> · concept:<why>');
      else res.tier = fr.source.split(':')[0];
      if (!/Frame \w+ · §/.test(fr.caption)) res.warnings.push('the caption does not end with "Frame <id> · §<section>" (which plan section uses it)');
      if (PLACEHOLDER.test(fr.caption.replace(/<[^>]+>/g, ' '))) res.warnings.push(`placeholder text in the caption: "${fr.caption.replace(/<[^>]+>/g, ' ').match(PLACEHOLDER)[0]}"`);
      if (fr.sidecar) {
        const sc = JSON.parse(fs.readFileSync(fr.sidecar, 'utf8'));
        if (sc.fontsFailed?.length) res.errors.push(`fonts failed to load in the capture: ${sc.fontsFailed.join(', ')} — the frame is not the real product's type`);
        if (sc.pageErrors?.length) res.warnings.push(`${sc.pageErrors.length} page error(s) during capture (e.g. "${sc.pageErrors[0].slice(0, 80)}") — fix the fixtures, or ignore them in the capture spec with a reason`);
        if (sc.unanswered?.length) res.warnings.push(`${sc.unanswered.length} API call(s) answered with {} (e.g. ${sc.unanswered[0]}) — add a fixture with the real response shape, or declare it empty (null)`);
        if (sc.missingFiles?.length) res.warnings.push(`${sc.missingFiles.length} build file(s) missing (e.g. ${sc.missingFiles[0]}) — the product build is incomplete`);
        if (sc.overflowX) res.warnings.push('the captured page scrolls sideways at this viewport');
        const pm = (sc.text || '').match(PLACEHOLDER);
        if (pm) res.warnings.push(`placeholder or artefact text on screen: "${pm[0]}"`);
      }
    }
    items.push({ name: `${id}${res.tier ? ` · ${res.tier}` : ''}`, png: path.join(shots, f), ...res });
  }
  await page.close();
  // Near-duplicates inside one gallery: two states that look the same teach the reader nothing.
  const byGallery = {};
  for (const [id, fr] of frames) (byGallery[fr.gallery] ||= []).push(id);
  for (const ids of Object.values(byGallery)) {
    const have = ids.filter((id) => thumbs[id]);
    const viewportFrames = have.filter((id) => ['capture', 'rebuild'].includes((frames.get(id).source || '').split(':')[0]));
    const dims = new Set(viewportFrames.map((id) => sizes[id]));
    if (dims.size > 1) { const it = items.find((x) => x.name.startsWith(have[0])); it?.warnings.push(`frames in ${frames.get(have[0]).gallery} have different sizes (${[...dims].join(', ')}) — one viewport per gallery unless a frame says why`); }
    for (let i = 0; i < have.length; i++) for (let j = i + 1; j < have.length; j++) {
      const a = thumbs[have[i]]; const c = thumbs[have[j]];
      if (a.length !== c.length) continue;
      const diff = a.reduce((n, v, k) => n + Math.abs(v - c[k]), 0) / a.length;
      if (diff < 0.35) items.find((x) => x.name.startsWith(have[j]))?.warnings.push(`looks almost identical to ${have[i]} — make the state difference visible, or drop one`);
    }
  }
  if (!files.length) items.push({ name: 'no mockup shots', png: shots, errors: [], warnings: ['mockups/shots/ is empty — run node build/build-plan.mjs --shots'] });
  const tiers = items.reduce((m, it) => { if (it.tier) m[it.tier] = (m[it.tier] || 0) + 1; return m; }, {});
  writeReport(dir, `Mockup review — ${Object.entries(tiers).map(([k, v]) => `${v} ${k}`).join(' · ') || 'no provenance recorded'}`, items, MOCKUP_RUBRIC);
  summarise('mockups', items);
}

// ---------------------------------------------------------------- deck
const DECK_RUBRIC = [
  'Each slide is a visual aid: one idea, little text, a diagram / mockup / number that the script talks over.',
  'Nothing overflows the 1280×720 stage; crops show whole, legible regions of the diagrams.',
  'The lifecycle / section markers make it obvious where in the story the audience is.',
  'Every number on a slide is in the master plan.',
];
async function reviewDeck(b) {
  const deckDir = path.join(ROOT, arg || 'presentation');
  const dir = path.join(OUT, 'deck');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const index = path.join(deckDir, 'index.html');
  if (!fs.existsSync(index)) throw new Error(`no ${path.relative(ROOT, index)} — build the deck first`);
  const ids = [...fs.readFileSync(index, 'utf8').matchAll(/<section class="slide[^"]*" data-id="([^"]+)"/g)].map((m) => m[1]);
  const page = await b.newPage();
  await page.setViewport({ width: 1280, height: 720, deviceScaleFactor: 1 });
  const items = [];
  let i = 0;
  for (const id of ids) {
    await page.goto(`file://${index}?s=${i}#${id}`, { waitUntil: 'load' });
    i += 1;
    await page.evaluate(() => document.fonts.ready);
    await sleep(700);
    const res = await page.evaluate((sid, font) => {
      const errors = []; const warnings = [];
      if (!document.fonts.check(`16px "${font}"`)) errors.push(`the ${font} font did not load`);
      const s = document.querySelector(`section.slide[data-id="${sid}"]`);
      if (!s || !s.classList.contains('is-active')) { errors.push('slide did not become active'); return { errors, warnings }; }
      const box = s.getBoundingClientRect();
      s.querySelectorAll('*').forEach((el) => {
        const r = el.getBoundingClientRect();
        if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden') return;
        if (r.right > box.right + 2 || r.bottom > box.bottom + 2 || r.left < box.left - 2 || r.top < box.top - 2) {
          const t = (el.textContent || el.tagName).trim().slice(0, 40);
          if (el.children.length === 0 || el.tagName === 'svg') errors.push(`"${t}" overflows the slide`);
        }
        if (el.scrollHeight > el.clientHeight + 4 && /hidden|clip/.test(getComputedStyle(el).overflowY) && el.clientHeight > 0 && el.tagName !== 'svg') {
          warnings.push(`content is clipped inside <${el.tagName.toLowerCase()} class="${el.className}">`);
        }
      });
      return { errors: [...new Set(errors)], warnings: [...new Set(warnings)] };
    }, id, FONT);
    const png = path.join(dir, `${String(i).padStart(2, '0')}-${id}.png`);
    await page.screenshot({ path: png });
    items.push({ name: `slide ${i} · ${id}`, png, ...res });
  }
  await page.close();
  writeReport(dir, 'Deck review', items, DECK_RUBRIC);
  summarise('deck', items);
}

// ---------------------------------------------------------------- mermaid twins
const MERMAID_RUBRIC = [
  'Each mermaid twin tells the same story as its draw.io diagram, simplified — same nodes, same order, same example.',
  'It renders cleanly: no overlapping labels, no bars starting at 0 in gantt charts, no self-loop labels on top of each other.',
  'Labels are short; long explanations live in the text, not in mermaid nodes.',
];
function reviewMermaid() {
  const dir = path.join(OUT, 'mermaid');
  fs.rmSync(dir, { recursive: true, force: true });
  const master = fs.readFileSync(path.join(ROOT, cfg.master), 'utf8');
  const res = renderMermaid(mermaidBlocks(master), dir);
  if (!res) { console.log('⚠ mermaid-cli not installed (npm install @mermaid-js/mermaid-cli) — mermaid twins not rendered'); return; }
  const items = res.map((r) => ({ name: `${r.id} (master line ${r.line})`, png: r.png, errors: r.error ? [`mermaid does not render: ${r.error}`] : [], warnings: [] }));
  writeReport(dir, 'Mermaid twin review', items, MERMAID_RUBRIC);
  summarise('mermaid', items);
}

// ---------------------------------------------------------------- anchors
// Resolution is mechanical; judging is not. The report puts each claim next to the code it cites,
// so the reader only has to answer "does this excerpt say what the sentence says?".
function reviewAnchors() {
  const dir = path.join(OUT, 'anchors');
  fs.mkdirSync(dir, { recursive: true });
  const files = [cfg.master];
  if (process.argv.includes('--evidence') && fs.existsSync(path.join(ROOT, 'evidence'))) files.push(...fs.readdirSync(path.join(ROOT, 'evidence')).filter((f) => f.endsWith('.md')).map((f) => `evidence/${f}`));
  const anchors = files.flatMap((f) => findAnchors(f, fs.readFileSync(path.join(ROOT, f), 'utf8')));
  const { repos, anchors: res } = resolveAnchors(cfg, anchors);
  const count = (st) => res.filter((r) => r.status === st).length;
  const lines = ['# Anchor review', '', `Generated ${new Date().toISOString().slice(0, 16).replace('T', ' ')} by build/review.mjs anchors — ${res.length} anchor(s) in ${files.join(', ')}.`, '',
    `**${count('ok')} ✔ resolved · ${count('weak')} ⚠ identifiers not at the lines · ${count('ambiguous')} ⚠ ambiguous · ${count('range')} ✘ past end of file · ${count('missing')} ✘ not found**`, '',
    'For every anchor: read the excerpt and the sentence together. The claim must be what the code does at these lines — the right symbol, the right behaviour, the full range and no more. Fix the master (tighten the range, correct the claim, or name the repo) and re-run until every anchor is ✔ and every excerpt reads true. Record the outcome in evidence/99-verification.md.', ''];
  if (!repos.length) lines.push('> ⚠ plan.config.mjs has no `repos`, so only anchors into analysis/ could resolve. Add every pinned repository: `repos: { \'<repo>\': { path, sha, base, watch } }`.', '');
  for (const r of repos) if (r.problem) lines.push(`> ✘ ${r.key}: ${r.problem}`, '');
  lines.push('## Moving ground since the pins', '');
  const byRepo = {};
  for (const a of res) for (const c of a.candidates) { const m = c.match(/^([^@]+)@[^:]+:(.+)$/); if (m) (byRepo[m[1]] ||= new Set()).add(m[2]); }
  for (const r of repos.filter((x) => !x.problem)) lines.push(...movingGroundMd(movingGround(r, [...(byRepo[r.key] || [])])));
  lines.push('## Anchors', '', ...anchorsReportMd({ anchors: res }));
  fs.writeFileSync(path.join(dir, 'REPORT.md'), lines.join('\n'));
  const errors = res.filter((r) => ['missing', 'range'].includes(r.status));
  totalErrors += errors.length;
  for (const r of res.filter((x) => x.status !== 'ok')) console.log(`${['missing', 'range'].includes(r.status) ? '✘' : '⚠'} ${r.file}:${r.line} \`${r.raw}\` — ${r.status}${r.notes.length ? `: ${r.notes.join('; ')}` : ''}`);
  console.log(`anchors: ${res.length} anchor(s), ${errors.length} error(s), ${count('weak') + count('ambiguous')} warning(s) → review/anchors/REPORT.md (read every excerpt against its sentence)`);
}

fs.mkdirSync(OUT, { recursive: true });
if (what === 'anchors') { reviewAnchors(); process.exit(totalErrors ? 1 : 0); }
if (what === 'mermaid' || what === 'all') reviewMermaid();
const b = await browser();
try {
  if (what === 'diagrams' || what === 'all') await reviewDiagrams(b);
  if (what === 'html' || what === 'all') await reviewHtml(b);
  if (what === 'mockups' || (what === 'all' && cfg.mockups.length)) await reviewMockups(b);
  if (what === 'deck' || (what === 'all' && fs.existsSync(path.join(ROOT, 'presentation', 'index.html')))) await reviewDeck(b);
  if (what === 'all' && cfg.repos) reviewAnchors();
  if (!['diagrams', 'html', 'mockups', 'deck', 'mermaid', 'all'].includes(what)) throw new Error(`unknown review target "${what}"`);
} finally {
  await b.close();
  for (const f of fs.readdirSync(OUT)) if (f.startsWith('.') && f.endsWith('.html')) fs.rmSync(path.join(OUT, f));
}
process.exit(totalErrors ? 1 : 0);
