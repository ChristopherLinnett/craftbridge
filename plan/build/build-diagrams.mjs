// Builds every diagram in diagrams/src/*.mjs:
//   1. writes the editable draw.io source  → diagrams/drawio/<id>.drawio
//   2. exports a transparent SVG via the draw.io Desktop CLI → diagrams/drawio/<id>.svg
//   3. prints the kit's lint (errors fail the build; fix them, don't suppress them)
// Usage: node build/build-diagrams.mjs [id-filter]
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { pathToFileURL } from 'node:url';
import { ROOT, loadConfig, drawioBin } from './lib/common.mjs';
import { LINT } from './drawio-kit.mjs';

const run = promisify(execFile);
const cfg = await loadConfig();
const srcDir = path.join(ROOT, cfg.diagramsSrc);
const outDir = path.join(ROOT, cfg.diagramsOut);
const filter = process.argv[2];

if (!fs.existsSync(srcDir)) { console.log(`no ${cfg.diagramsSrc}/ — no kit diagrams to build`); process.exit(0); }
fs.mkdirSync(outDir, { recursive: true });
const files = fs.readdirSync(srcDir).filter((f) => f.endsWith('.mjs') && (!filter || f.includes(filter))).sort();
if (!files.length) { console.log(`no diagram sources in ${cfg.diagramsSrc}${filter ? ` matching "${filter}"` : ''}`); process.exit(0); }

const DRAWIO = drawioBin();
let failed = 0;
for (const file of files) {
  const mod = await import(pathToFileURL(path.join(srcDir, file)).href);
  const { id, xml } = mod.default;
  if (!id || !xml) throw new Error(`${file}: default export must be { id, xml }`);
  if (`${id}.mjs` !== file) console.log(`⚠ ${file}: id "${id}" differs from the file name — keep them equal`);
  const drawioPath = path.join(outDir, `${id}.drawio`);
  const svgPath = path.join(outDir, `${id}.svg`);
  fs.writeFileSync(drawioPath, xml);
  // --theme light keeps authored colours verbatim (auto would wrap them in light-dark()).
  await run(DRAWIO, ['-x', '-f', 'svg', '-t', '--theme', 'light', '--embed-svg-fonts', 'false', '--no-sandbox', '-o', svgPath, drawioPath]);
  // Drop the per-label PNG fallbacks draw.io adds inside <switch>: every target browser renders
  // the foreignObject HTML labels, and the fallbacks are ~95% of the file size.
  const svg = fs.readFileSync(svgPath, 'utf8').replace(/<image [^>]*xlink:href="data:image\/png;base64,[^"]*"[^>]*\/>/g, '');
  fs.writeFileSync(svgPath, svg);
  const lint = LINT.get(id) || { errors: [], warnings: [] };
  console.log(`${id}: ${(fs.statSync(svgPath).size / 1024).toFixed(1)} KB svg${lint.errors.length ? ` · ✘ ${lint.errors.length} error(s)` : ''}${lint.warnings.length ? ` · ⚠ ${lint.warnings.length} warning(s)` : ''}`);
  for (const e of lint.errors) console.log(`   ✘ ${e}`);
  for (const w of lint.warnings) console.log(`   ⚠ ${w}`);
  if (lint.errors.length) failed += 1;
}
if (failed) { console.error(`\n✘ ${failed} diagram(s) failed lint — fix the layout, then run node build/review.mjs diagrams`); process.exit(1); }
console.log('\nNext: node build/review.mjs diagrams   (renders PNGs + rendered checks; then LOOK at every PNG)');
