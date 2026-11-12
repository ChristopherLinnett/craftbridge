/* ============================================================================
   scoping-plan deck assembler (kit/deck/build-deck.mjs, installed at <plan>/build/deck/)

   One implementation for every plan deck. A deck folder holds only its content
   (deck.config.mjs, slides/<ID>.html, SCRIPT.md, autopilot-data.js, audio) and a
   two-line build-deck.mjs that calls buildDeck(config). This module:

     1. installs the runtime (framework.css themed from theme.json, deck.js,
        autopilot.js, vendor/) into the deck folder, so every deck stays
        self-contained and offline, with one source of truth in the skill's kit;
     2. validates each fragment (one <section>, id/type match the configured
        order, no <script>/<style>, token-only colours);
     3. resolves placeholders:
          <div class="diagram" data-diagram="NAME" [data-crop="x y w h"]></div>
              → the SVG found in config.diagramDirs, inlined (optionally cropped
                by rewriting its viewBox, so a slide can zoom into one region of
                a plan diagram without a second copy of it)
          <div class="demo-slot"></div>
              → iframe slot for config.demoSrc (deck.js mounts/unmounts it)
          src="../path/to/image.png" (media outside the deck folder)
              → copied into <deck>/assets/ and rewritten, so the deck folder can
                be zipped and presented anywhere;
     4. writes index.html from template.html (tailwind colours + font from theme.json).

          <ol class="life-track" data-current="Step"></ol>
              → the deck's lifecycle stepper from config.lifeTrack
   config: { root, title, order: [[id, type], …], diagramDirs: [dir, …],
             demoSrc?: 'demo.html', lifeTrack?: [step, …], types?: [...extra data-type values] }
   ========================================================================== */

import { readFileSync, writeFileSync, existsSync, mkdirSync, cpSync, copyFileSync, statSync } from 'node:fs';
import { dirname, join, resolve, basename, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { THEME, TOKEN_HEXES, applyTheme } from '../lib/common.mjs';

const KIT = dirname(fileURLToPath(import.meta.url));
const VENDOR = join(KIT, '..', 'vendor');
const RUNTIME = ['framework.css', 'deck.js', 'autopilot.js', 'vendor'];

const BASE_TYPES = ['title', 'divider', 'data', 'content', 'diagram', 'mockup-hero', 'mockup', 'demo', 'close'];

/* Token hexes come from theme.json (lib/common.mjs TOKEN_HEXES). Anything else in a fragment is flagged. */

export function buildDeck(config) {
  const root = config.root;
  const errors = [];
  const warnings = [];
  const types = new Set([...BASE_TYPES, ...(config.types || [])]);

  /* ---------------- 1. install the shared runtime ---------------- */
  writeFileSync(join(root, 'framework.css'), applyTheme(readFileSync(join(KIT, 'runtime', 'framework.css'), 'utf8')));
  for (const name of ['deck.js', 'autopilot.js']) copyFileSync(join(KIT, 'runtime', name), join(root, name));
  cpSync(VENDOR, join(root, 'vendor'), { recursive: true });
  writeFileSync(join(root, 'KIT-INSTALLED.md'),
    `# Installed by the scoping-plan kit — do not edit\n\n${RUNTIME.map((n) => `- \`${n}\``).join('\n')}\n\n` +
    `These are written by \`build-deck.mjs\` on every build from \`${relative(root, KIT)}/runtime/\` and \`${relative(root, VENDOR)}/\` ` +
    '(themed from theme.json). Change the framework in the scoping-plan skill\'s kit/, then reinstall the kit; never edit these copies.\n');
  if (!existsSync(join(root, 'autopilot-data.js'))) {
    writeFileSync(join(root, 'autopilot-data.js'),
      '/* Per-slide narration table. Empty = no autopilot button; the deck is unchanged.\n' +
      '   Add one line per recorded slide: { id: \'<slide id>\', src: \'audio/<id>.mp3\', dur: <seconds> } */\n' +
      'window.AUTOPILOT_DATA = { buffer: { advance: 700, lead: 1000, tail: 1000, fallback: 8000, guard: 2500 }, slides: [] };\n');
  }

  /* ---------------- 2. validation ---------------- */
  function validate(id, type, frag) {
    const sections = frag.match(/<section\b/g) || [];
    if (sections.length !== 1) errors.push(`slides/${id}.html: expected exactly 1 <section>, found ${sections.length}`);
    const secTag = (frag.match(/<section\b[^>]*>/) || [''])[0];
    if (!new RegExp(`data-id=["']${id}["']`).test(secTag)) errors.push(`slides/${id}.html: data-id does not match "${id}"`);
    const t = (secTag.match(/data-type=["']([^"']+)["']/) || [])[1];
    if (!t) errors.push(`slides/${id}.html: missing data-type`);
    else if (!types.has(t)) errors.push(`slides/${id}.html: unknown data-type "${t}"`);
    else if (t !== type) errors.push(`slides/${id}.html: data-type "${t}" but the deck order says "${type}"`);
    if (/<script\b/i.test(frag)) errors.push(`slides/${id}.html: <script> is forbidden in fragments`);
    if (/<style\b/i.test(frag)) errors.push(`slides/${id}.html: <style> is forbidden — use framework classes`);
    if (/font-family\s*:/i.test(frag)) warnings.push(`slides/${id}.html: font-family override — use framework classes`);
    const hexes = frag.match(/(?<!&#)(?<![\w#])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3})\b/g) || [];
    for (const h of hexes) if (!TOKEN_HEXES.has(h.toLowerCase())) warnings.push(`slides/${id}.html: non-token hex "${h}"`);
  }

  /* ---------------- 3. placeholders ---------------- */
  function findDiagram(name) {
    for (const dir of config.diagramDirs || ['diagrams']) {
      const p = resolve(root, dir, `${name}.svg`);
      if (existsSync(p)) return p;
    }
    return null;
  }
  function resolveDiagrams(id, frag) {
    return frag.replace(
      /<div\b(?=[^>]*\bclass="[^"]*\bdiagram\b[^"]*")(?=[^>]*\bdata-diagram="([^"]+)")[^>]*>\s*<\/div>/g,
      (m, name) => {
        const p = findDiagram(name);
        if (!p) { errors.push(`slides/${id}.html: diagram "${name}" not found in ${config.diagramDirs}`); return m; }
        let svg = readFileSync(p, 'utf8').replace(/^\s*<\?xml[^>]*\?>\s*/i, '').replace(/^\s*<!DOCTYPE[^>]*>\s*/i, '').trim();
        const crop = (m.match(/data-crop="([^"]+)"/) || [])[1];
        if (crop) {
          // Zoom into one region: new viewBox, and an intrinsic size equal to the crop so the
          // SVG scales like an image (a box of any other aspect would letterbox and reveal
          // content outside the crop).
          const [, , cw, ch] = crop.split(/\s+/);
          svg = svg.replace(/<svg\b([^>]*)>/, (tag, attrs) =>
            `<svg${attrs.replace(/\swidth="[^"]*"/, ` width="${cw}px"`).replace(/\sheight="[^"]*"/, ` height="${ch}px"`).replace(/viewBox="[^"]*"/, `viewBox="${crop}"`)}>`);
        }
        const cls = (m.match(/class="([^"]*)"/) || [])[1];
        return `<div class="${cls}" data-diagram="${name}"${crop ? ` data-crop="${crop}"` : ''}>${svg}</div>`;
      });
  }
  function resolveDemo(frag) {
    return frag.replace(/<div\b(?=[^>]*\bclass="[^"]*\bdemo-slot\b[^"]*")[^>]*>\s*<\/div>/g, () => {
      const src = config.demoSrc || 'demo.html';
      return `<div class="demo-slot" data-demo-src="${src}">` +
        `<iframe class="demo-iframe" src="${src}" frameborder="0" allow="autoplay" tabindex="-1" aria-hidden="true"></iframe></div>`;
    });
  }
  // <ol class="life-track" data-current="Record"></ol> → the deck's lifecycle steps
  // (config.lifeTrack), earlier ones done, the named one(s) current. One definition per deck.
  function resolveLifeTrack(id, frag) {
    return frag.replace(/<ol class="life-track" data-current="([^"]+)"><\/ol>/g, (m, cur) => {
      const steps = config.lifeTrack || [];
      const current = cur.split(',').map((c) => c.trim());
      const at = Math.min(...current.map((c) => steps.indexOf(c)));
      if (!steps.length || current.some((c) => !steps.includes(c))) {
        errors.push(`slides/${id}.html: life-track step "${cur}" not in config.lifeTrack`);
        return m;
      }
      const items = steps.map((st, i) =>
        `<li class="${current.includes(st) ? 'is-current' : i < at ? 'is-done' : ''}">${i + 1} · ${st}</li>`).join('');
      return `<ol class="life-track" aria-label="Lifecycle">${items}</ol>`;
    });
  }
  function localiseAssets(id, frag) {
    // Media only (images/video): an HTML page would lose its own relative references if copied.
    return frag.replace(/\bsrc="(\.\.\/[^"]+\.(?:png|jpe?g|webp|gif|svg|mp4|webm))"/gi, (m, rel) => {
      const from = resolve(root, rel);
      if (!existsSync(from)) { errors.push(`slides/${id}.html: asset missing ${rel}`); return m; }
      mkdirSync(join(root, 'assets'), { recursive: true });
      const to = join(root, 'assets', basename(from));
      if (!existsSync(to) || statSync(to).mtimeMs < statSync(from).mtimeMs) copyFileSync(from, to);
      return `src="assets/${basename(from)}"`;
    });
  }

  /* ---------------- 4. assemble ---------------- */
  const template = readFileSync(join(KIT, 'template.html'), 'utf8');
  const fragments = [];
  for (const [id, type] of config.order) {
    const file = join(root, 'slides', `${id}.html`);
    if (!existsSync(file)) { warnings.push(`slides/${id}.html missing — skipped`); continue; }
    let frag = readFileSync(file, 'utf8');
    validate(id, type, frag);
    frag = localiseAssets(id, resolveLifeTrack(id, resolveDemo(resolveDiagrams(id, frag))));
    fragments.push(`<!-- slide ${id} -->\n${frag.trim()}`);
  }
  if (!fragments.length) { console.error('✘ no slide fragments found'); process.exit(1); }

  const tw = THEME.deck?.tailwindColors || {};
  const twColors = `{\n${Object.entries(tw).map(([k, v]) => `      ${k}:'${v}'`).join(',\n')}\n    }`;
  const out = template
    .replace('{{FONT_SANS}}', THEME.font.sans)
    .replace('{{TAILWIND_COLORS}}', twColors)
    .replace('{{TITLE}}', config.title)
    .replace('{{COUNT}}', String(fragments.length))
    .replace('{{SLIDES}}', fragments.join('\n\n'));
  writeFileSync(join(root, 'index.html'), out, 'utf8');

  console.log(`build-deck: ${fragments.length}/${config.order.length} slides → ${join(root, 'index.html')} (${(out.length / 1024).toFixed(1)} KB)`);
  if (warnings.length) { console.log(`\n⚠ ${warnings.length} warning(s):`); warnings.forEach((w) => console.log(`  · ${w}`)); }
  if (errors.length) { console.error(`\n✘ ${errors.length} error(s):`); errors.forEach((e) => console.error(`  · ${e}`)); process.exit(1); }
  console.log('✔ build OK');
}
