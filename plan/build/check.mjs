// Plan lint — the mechanical half of the writing rules. Run after every section and before sign-off.
//   node build/check.mjs            errors fail; pending sections / open questions are warnings
//   node build/check.mjs --final    pending sections, open questions and stale views become errors
// Checks: meta/process language · placeholders · mermaid↔draw.io pairing and parity · diagram/mockup
// references · every register (PD, D-, L, E, R, …) numbered in order with no gaps, every reference
// resolving · frame prefixes that collide with a register · frame references and flow order · header
// counts · pins and code anchors against plan.config.mjs repos · superseded wording · local links ·
// stale generated views · deck + script rules (script sections per slide, spoken prose, every number
// traceable to the master).
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, loadConfig, exists } from './lib/common.mjs';
import { mermaidBlocks as twinBlocks, twinCounts, drawioCounts, parityProblems } from './lib/mermaid.mjs';

const cfg = await loadConfig();
const FINAL = process.argv.includes('--final');
const errors = [];
const warnings = [];
const err = (f, l, m) => errors.push(`${f}${l ? `:${l}` : ''}  ${m}`);
const warn = (f, l, m) => warnings.push(`${f}${l ? `:${l}` : ''}  ${m}`);
const master = fs.readFileSync(path.join(ROOT, cfg.master), 'utf8');

// ---------------------------------------------------------------- 1. meta / process language
// A plan states intent. It never narrates how it was made, what an earlier draft said, what the
// user answered, or that something "turned out". Slides and scripts never talk about themselves.
const META = [
  [/\b(?:earlier|first|previous|original|old) (?:draft|version) of (?:the|this) (?:plan|document|doc)\b/i, 'plan history — state the current intent only'],
  [/\b(?:the|this) (?:draft|v\d(?:\.\d)?) (?:said|says|proposed|had)\b/i, 'plan history — state the current intent only'],
  [/\bv\d\.\d (?:said|had|proposed)\b|\bv\d\.\d → v\d\.\d\b/i, 'plan history'],
  [/\b(?:it )?turn(?:s|ed) out\b/i, 'discovery narrative — state the fact'],
  [/\bwe (?:found|discovered|realised|realized|noticed|verified|checked|confirmed|looked)\b/i, 'process narrative — state the fact and cite its evidence'],
  [/\balready (?:taken|in use|used by)\b/i, 'complaint about an unusable option — use the alternative and state it plainly'],
  [/\b(?:the )?(?:operator|user) (?:answered|said|asked|wanted|chose|confirmed|decided|requested)\b/i, 'conversation attribution — record the decision as a PD, not who said it'],
  // "per the user", "as per", "per your request" — but not "one row per user".
  [/\bas per\b|\bper (?:the|your) (?:operator|user|request|conversation|instructions?)\b|\bper (?:request|instructions?)\b|\bas requested\b/i, 'conversation attribution'],
  [/\bOperator\b/, 'conversation framing ("Operator")'],
  [/\bopen questions?\b/i, 'open-question framing — the plan records decisions; questions live in PLAN-STATE.json'],
  [/\b(?:corrections?|errata) (?:to|from|of) (?:the|this|v\d)/i, 'plan history'],
  [/\bno longer (?:proposes|says|uses the)\b/i, 'plan history'],
  [/\b(?:TBD|TODO|FIXME|XXX)\b|\?\?\?|lorem ipsum/i, 'placeholder'],
];
const SLIDE_META = [
  [/\b(?:this|the next|the previous|on this|in this) slide\b/i, 'the script talks about the plan, never about the slide'],
  [/\bthe slide (?:shows|says|lists|has)\b/i, 'the script talks about the plan, never about the slide'],
  [/\bas you can see\b/i, 'narrating the visual — explain the plan instead'],
  [/\bthe plan (?:looked|looks|goes) (?:for|at|into)\b/i, 'narrating the plan process'],
  [/\b(?:this|the) (?:presentation|deck) (?:is about|covers|will)\b/i, 'meta framing'],
];
function scan(file, text, rules, { skipCode = true } = {}) {
  let inCode = false;
  text.split('\n').forEach((line, i) => {
    if (/^```/.test(line)) { inCode = !inCode; return; }
    if (skipCode && inCode) return;
    const plain = line.replace(/`[^`]*`/g, '').replace(/<!--[\s\S]*?-->/g, '');
    for (const [re, why] of rules) { const m = plain.match(re); if (m) err(file, i + 1, `"${m[0]}" — ${why}`); }
  });
}
scan(cfg.master, master, META);

// ---------------------------------------------------------------- 2. pending sections
const pend = [...master.matchAll(/<!-- @pending ([\s\S]*?) -->/g)];
for (const p of pend) (FINAL ? err : warn)(cfg.master, master.slice(0, p.index).split('\n').length, `pending: ${p[1].slice(0, 80)}`);

// ---------------------------------------------------------------- 3. mermaid ↔ draw.io, diagrams, mockups
const lineOf = (idx) => master.slice(0, idx).split('\n').length;
const tagged = new Set();
for (const m of master.matchAll(/```mermaid\n(.*)\n/g)) {
  const t = m[1].match(/^%% @drawio (\S+) \| (.+)$/);
  if (!t) { err(cfg.master, lineOf(m.index), 'mermaid block without "%% @drawio <id> | <caption>" — every diagram has a draw.io twin and a caption'); continue; }
  const id = t[1];
  tagged.add(id);
  const src = path.join(ROOT, cfg.diagramsSrc, `${id}.mjs`);
  const svg = path.join(ROOT, cfg.diagramsOut, `${id}.svg`);
  if (!fs.existsSync(src)) err(cfg.master, lineOf(m.index), `diagram ${id}: no ${cfg.diagramsSrc}/${id}.mjs`);
  else if (!fs.existsSync(svg)) err(cfg.master, lineOf(m.index), `diagram ${id}: not built — node build/build-diagrams.mjs ${id}`);
  else if (fs.statSync(svg).mtimeMs < fs.statSync(src).mtimeMs) warn(cfg.master, lineOf(m.index), `diagram ${id}: SVG older than its source — rebuild and re-review`);
  const rev = path.join(ROOT, 'review', 'diagrams', `${id}.png`);
  if (fs.existsSync(svg) && (!fs.existsSync(rev) || fs.statSync(rev).mtimeMs < fs.statSync(svg).mtimeMs)) warn(cfg.master, lineOf(m.index), `diagram ${id}: not reviewed since its last build — node build/review.mjs diagrams ${id}`);
}
if (exists(cfg.diagramsSrc)) {
  for (const f of fs.readdirSync(path.join(ROOT, cfg.diagramsSrc)).filter((x) => x.endsWith('.mjs'))) {
    const id = f.replace(/\.mjs$/, '');
    if (!tagged.has(id)) warn(`${cfg.diagramsSrc}/${f}`, 0, 'diagram is not placed in the master (no %% @drawio tag)');
  }
}
for (const m of master.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)) {
  if (!/^https?:/.test(m[1]) && !exists(m[1])) err(cfg.master, lineOf(m.index), `image not found: ${m[1]}`);
}
for (const mk of cfg.mockups) {
  if (!exists(mk.file)) { err('plan.config.mjs', 0, `mockup gallery missing: ${mk.file}`); continue; }
  const ids = [...fs.readFileSync(path.join(ROOT, mk.file), 'utf8').matchAll(/<figure class="frame"[^>]*\bid="([^"]+)"/g)].map((x) => x[1]);
  for (const id of ids) if (!master.includes(`mockups/shots/${id}.png`)) warn(mk.file, 0, `frame ${id} is not shown in the master`);
}

// Twin parity: the mermaid twin tells the same story as its draw.io diagram.
for (const b of twinBlocks(master)) {
  const xml = path.join(ROOT, cfg.diagramsOut, `${b.id}.drawio`);
  if (!fs.existsSync(xml)) continue;
  for (const pr of parityProblems(twinCounts(b.code), drawioCounts(fs.readFileSync(xml, 'utf8')))) warn(cfg.master, b.line, `diagram ${b.id}: ${pr}`);
}

if (FINAL) {
  const { mermaidBlocks, renderMermaid } = await import('./lib/mermaid.mjs');
  const res = renderMermaid(mermaidBlocks(master), path.join(ROOT, 'review', '.mermaid-check'));
  if (!res) warn(cfg.master, 0, 'mermaid-cli not installed — mermaid twins not validated (npm install)');
  else for (const r of res) if (r.error) err(cfg.master, r.line, `mermaid ${r.id} does not render: ${r.error}`);
  fs.rmSync(path.join(ROOT, 'review', '.mermaid-check'), { recursive: true, force: true });
}

// ---------------------------------------------------------------- 4. registers + cross-references
// A register is any id family defined as the first cell of table rows: PD1…, D-1…, L1…, E1…, BE-1…,
// W0…. Each is numbered in document order with no gaps or duplicates, and every reference resolves.
const proseLines = [];
{ let inCode = false; master.split('\n').forEach((line, i) => { if (/^```/.test(line)) { inCode = !inCode; return; } if (!inCode) proseLines.push([i + 1, line]); }); }
// References are read from prose only: code spans hold identifiers, not register ids.
const refLines = proseLines.map(([ln, line]) => [ln, line.replace(/`[^`]*`/g, ' ').replace(/<!--[\s\S]*?-->/g, ' ')]);
const registers = new Map();                     // prefix → [{ n, raw, line }]
for (const [ln, line] of proseLines) {
  const m = line.match(/^\| \*{0,2}([A-Z]{1,3}-?)(\d+)([a-z]?)\*{0,2}(?= \||\s·|\s—|:)/);
  if (!m) continue;
  (registers.get(m[1]) || registers.set(m[1], []).get(m[1])).push({ n: +m[2], raw: `${m[1]}${m[2]}${m[3]}`, line: ln, suffix: m[3] });
}
for (const [k, rows] of [...registers]) if (rows.length < 2) registers.delete(k);
const defined = (prefix, n) => registers.get(prefix)?.some((r) => r.n === n);
for (const [prefix, rows] of registers) {
  // Only the register's own table counts for order: rows that repeat ids elsewhere (a task list citing E-rows) are references.
  const first = new Map();
  for (const r of rows) if (!first.has(r.n)) first.set(r.n, r);
  const seqRows = [...first.values()];
  const base = seqRows[0].n === 0 ? 0 : 1;
  seqRows.forEach((r, i) => {
    if (r.suffix) warn(cfg.master, r.line, `${r.raw}: suffix numbering in the ${prefix} register — renumber the rows in order`);
    if (r.n !== base + i) err(cfg.master, r.line, `${prefix} register out of order or with a gap: ${r.raw} where ${prefix}${base + i} belongs — keep rows in numeric order with no gaps`);
  });
  // Duplicates: the same id twice inside one table (contiguous rows). Other tables may cite it.
  const tables = [];
  for (const r of rows) { const t = tables.at(-1); if (t && r.line - t.at(-1).line <= 2) t.push(r); else tables.push([r]); }
  for (const t of tables) t.forEach((r, i) => { if (t.findIndex((x) => x.raw === r.raw) !== i) err(cfg.master, r.line, `${r.raw} is defined twice in the ${prefix} register`); });
}
const pdDefined = new Set((registers.get('PD') || []).map((r) => `PD${r.n}`));
const dDefined = new Set((registers.get('D-') || []).map((r) => `D-${r.n}`));
const secDefined = new Set([...master.matchAll(/^#{2,3} (\d+(?:\.\d+)?)\.? /gm)].map((m) => m[1]));
if (!pdDefined.size && /\bPD\d+\b/.test(master)) warn(cfg.master, 0, 'PD citations exist but the product-decision register is empty — record the answered questions as PD rows (§ Product decisions)');
if (exists('PLAN-STATE.json')) {
  const answeredPd = new Set(JSON.parse(fs.readFileSync(path.join(ROOT, 'PLAN-STATE.json'), 'utf8')).questions.filter((q) => q.pd).map((q) => q.pd));
  for (const pd of answeredPd) if (pdDefined.size && !pdDefined.has(pd)) err(cfg.master, 0, `${pd} was recorded for an answered question but has no row in the register`);
}
const regRe = [...registers.keys()].sort((a, b) => b.length - a.length).map((k) => k.replace('-', '\\-')).join('|');
for (const [ln, line] of refLines) {
  if (regRe) for (const m of line.matchAll(new RegExp(`(?<![\\w-])(${regRe})(\\d+)\\b(?![-\\w])`, 'g'))) {
    const pre = line.slice(Math.max(0, m.index - 2), m.index);
    if (/[§#/.]$/.test(pre)) continue;
    if (!defined(m[1], +m[2])) err(cfg.master, ln, `${m[1]}${m[2]} is not in the ${m[1]} register`);
  }
  for (const m of line.matchAll(/§(\d+(?:\.\d+)?)/g)) if (secDefined.size && !secDefined.has(m[1]) && !secDefined.has(m[1].split('.')[0])) err(cfg.master, ln, `§${m[1]} does not exist`);
}
// Header counts ("45 product decisions") must match the registers.
for (const [re, set, what] of [[/\b(\d+) product decisions\b/g, pdDefined, 'product decisions'], [/\b(\d+) engineering decisions\b/g, dDefined, 'engineering decisions']]) {
  for (const [ln, line] of proseLines) for (const m of line.matchAll(re)) if (set.size && +m[1] !== set.size) err(cfg.master, ln, `says ${m[1]} ${what}; the register has ${set.size}`);
}

// ---------------------------------------------------------------- 4b. mockup frames
// Frame ids are the gallery prefix + number (S3, W1). A prefix equal to a register prefix makes "W1"
// ambiguous (wave or frame?); every frame the text cites exists; frames appear in flow order.
const frameIds = new Map();                      // prefix → Set(ids)
for (const mk of cfg.mockups) {
  if (!exists(mk.file)) continue;
  const html = fs.readFileSync(path.join(ROOT, mk.file), 'utf8');
  const ids = [...html.matchAll(/<figure class="frame"([^>]*)>/g)].map((x) => ({ id: (x[1].match(/\bid="([^"]+)"/) || [])[1], src: (x[1].match(/\bdata-source="([^"]*)"/) || [])[1] })).filter((x) => x.id);
  frameIds.set(mk.prefix, new Set(ids.map((x) => x.id)));
  const P = mk.prefix.toUpperCase();
  if (registers.has(P)) err('plan.config.mjs', 0, `mockup prefix "${mk.prefix}" (${mk.file}) collides with the ${P} register (${P}${registers.get(P)[0].n}…${P}${registers.get(P).at(-1).n}) — "${P}1" is ambiguous; give the gallery another prefix and renumber its frames`);
  for (const x of ids) if (!x.src) warn(mk.file, 0, `frame ${x.id} has no data-source (capture:<repo>@<sha> · template:<path> · provider:<what> · rebuild:<ui-spec> · concept:<why>)`);
  if (!registers.has(P)) {
    for (const [ln, line] of refLines) for (const m of line.matchAll(new RegExp(`(?<![\\w/.-])${P}(\\d+)\\b(?![-\\w.])`, 'g'))) {
      if (!frameIds.get(mk.prefix).has(`${mk.prefix}${m[1]}`)) err(cfg.master, ln, `${P}${m[1]} is cited but ${mk.file} has no frame ${mk.prefix}${m[1]}`);
    }
  }
  // Flow order: the order the master places the shots is the order the frames are numbered.
  const placed = [...master.matchAll(new RegExp(`mockups/shots/${mk.prefix}(\\d+)\\.png`, 'g'))].map((m) => +m[1]);
  const firsts = placed.filter((n, i) => placed.indexOf(n) === i);
  const outOfOrder = firsts.find((n, i) => i && n < firsts[i - 1]);
  if (outOfOrder != null) warn(cfg.master, 0, `${P} frames are placed out of numeric order (${firsts.map((n) => P + n).join(' → ')}) — number frames in the order the plan shows them`);
}

// ---------------------------------------------------------------- 4c. pins, anchors, superseded wording
if (cfg.repos) {
  for (const [k, r] of Object.entries(cfg.repos)) if (!master.includes(r.sha)) err(cfg.master, 0, `the pin ${k}@${r.sha} (plan.config.mjs) is not stated in the master — the header lists every repo with its SHA`);
  const { findAnchors, resolveAnchors } = await import('./lib/anchors.mjs');
  const { repos, anchors } = resolveAnchors(cfg, findAnchors(cfg.master, master));
  for (const r of repos) if (r.problem) (FINAL ? err : warn)('plan.config.mjs', 0, `${r.key}: ${r.problem}`);
  for (const a of anchors) if (['missing', 'range'].includes(a.status)) (FINAL ? err : warn)(cfg.master, a.line, `anchor \`${a.raw}\` ${a.status === 'missing' ? 'resolves to no file in the pinned repos or analysis/' : a.notes[0]} — node build/review.mjs anchors`);
  const rep = path.join(ROOT, 'review', 'anchors', 'REPORT.md');
  if (FINAL && (!fs.existsSync(rep) || fs.statSync(rep).mtimeMs < fs.statSync(path.join(ROOT, cfg.master)).mtimeMs)) warn(cfg.master, 0, 'anchors not reviewed since the master changed — node build/review.mjs anchors, then read every excerpt');
}
if (exists('PLAN-STATE.json')) {
  for (const q of JSON.parse(fs.readFileSync(path.join(ROOT, 'PLAN-STATE.json'), 'utf8')).questions.filter((x) => x.supersedes?.length)) {
    for (const [ln, line] of proseLines) for (const term of q.supersedes) {
      if (line.toLowerCase().includes(term.toLowerCase())) warn(cfg.master, ln, `"${term}" is wording ${q.pd || q.id} replaced — update the sentence to the decision`);
    }
  }
}

// ---------------------------------------------------------------- 5. links + stale views
const generated = new Set([cfg.html, ...[...master.matchAll(/<!-- @view ([^>]+?) -->/g)].flatMap((m) => m[1].split(/\s+/))]);
for (const m of master.matchAll(/\]\((?!https?:|#|mailto:)([^)\s#]+)(?:#[^)]*)?\)/g)) {
  const target = m[1].replace(/^\.\//, '');
  if (exists(target)) continue;
  if (generated.has(target)) (FINAL ? err : warn)(cfg.master, lineOf(m.index), `${target} is generated but not built yet — node build/build-plan.mjs`);
  else err(cfg.master, lineOf(m.index), `broken link: ${m[1]}`);
}
const mTime = fs.statSync(path.join(ROOT, cfg.master)).mtimeMs;
const viewFiles = new Set([...master.matchAll(/<!-- @view ([^>]+?) -->/g)].flatMap((m) => m[1].split(/\s+/)));
for (const v of viewFiles) {
  if (!exists(v)) (FINAL ? err : warn)(v, 0, 'generated view missing — node build/build-plan.mjs');
  else if (fs.statSync(path.join(ROOT, v)).mtimeMs < mTime - 1000) (FINAL ? err : warn)(v, 0, 'generated view older than the master — node build/build-plan.mjs');
}
if (exists(cfg.html) && fs.statSync(path.join(ROOT, cfg.html)).mtimeMs < mTime - 1000) (FINAL ? err : warn)(cfg.html, 0, 'rendered HTML older than the master — node build/build-plan.mjs');

// ---------------------------------------------------------------- 6. questions (PLAN-STATE.json)
if (exists('PLAN-STATE.json')) {
  const st = JSON.parse(fs.readFileSync(path.join(ROOT, 'PLAN-STATE.json'), 'utf8'));
  const open = (st.questions || []).filter((q) => q.status === 'open');
  for (const q of open) (FINAL ? err : warn)('PLAN-STATE.json', 0, `${q.id} unanswered: ${q.text.slice(0, 90)} — ask the user; never assume`);
}

// ---------------------------------------------------------------- 7. deck + script
const deckDir = path.join(ROOT, 'presentation');
if (fs.existsSync(path.join(deckDir, 'deck.config.mjs'))) {
  const { pathToFileURL } = await import('node:url');
  const deck = (await import(pathToFileURL(path.join(deckDir, 'deck.config.mjs')).href)).default;
  const norm = (s) => s.replace(/(\d),(\d{3})/g, '$1$2').replace(/−/g, '-');
  const masterNums = norm(master);
  const numbersIn = (text) => [...norm(text.replace(/<[^>]+>/g, ' ')).matchAll(/(?<![\w#.-])-?\d+(?:\.\d+)?(?=\s?%|\b)/g)]
    .map((m) => m[0]).filter((n) => n.replace('-', '').length >= 2 || /\./.test(n));
  for (const [id] of deck.order) {
    const f = path.join(deckDir, 'slides', `${id}.html`);
    if (!fs.existsSync(f)) { err(`presentation/slides/${id}.html`, 0, 'slide in deck.config.mjs has no fragment'); continue; }
    const html = fs.readFileSync(f, 'utf8');
    const visible = html.replace(/<(?:svg|code)[\s\S]*?<\/(?:svg|code)>/g, ' ').replace(/\b(?:class|style|data-[\w-]+|href|src)="[^"]*"/g, '');
    scan(`presentation/slides/${id}.html`, visible.replace(/<[^>]+>/g, ' '), [...META, ...SLIDE_META]);
    for (const n of new Set(numbersIn(visible))) if (!masterNums.includes(n)) warn(`presentation/slides/${id}.html`, 0, `number ${n} is not in the master — every figure must trace to the plan`);
  }
  const scriptPath = path.join(deckDir, 'SCRIPT.md');
  if (!fs.existsSync(scriptPath)) (FINAL ? err : warn)('presentation/SCRIPT.md', 0, 'no presenter script');
  else {
    const script = fs.readFileSync(scriptPath, 'utf8');
    scan('presentation/SCRIPT.md', script.replace(/^[^\n]*\n[^\n]*\n[^\n]*/, ''), [...META, ...SLIDE_META]);
    const secs = [...script.matchAll(/^## Slide (\d+) of (\d+) · `([^`]+)` · (.+)$/gm)];
    const ids = secs.map((s) => s[3]);
    const want = deck.order.map(([id]) => id);
    if (ids.join('|') !== want.join('|')) err('presentation/SCRIPT.md', 0, `script sections [${ids.join(', ')}] must match the deck order [${want.join(', ')}] one-to-one`);
    secs.forEach((s, i) => {
      const start = s.index + s[0].length;
      const end = i + 1 < secs.length ? secs[i + 1].index : script.length;
      const body = script.slice(start, end).replace(/^\*≈[^*]*\*$/m, '').replace(/^---$/m, '');
      const words = (body.match(/[A-Za-z0-9’'-]+/g) || []).length;
      const line = script.slice(0, s.index).split('\n').length;
      if (/^\s*(?:[-*+]|\d+\.)\s/m.test(body)) err('presentation/SCRIPT.md', line, `slide ${s[3]}: bullet list — the script is spoken prose`);
      const type = (deck.order.find(([id]) => id === s[3]) || [])[1];
      const min = ['divider', 'title', 'close'].includes(type) ? 15 : 60;
      if (words < min) err('presentation/SCRIPT.md', line, `slide ${s[3]}: ${words} words — too thin to explain this part of the plan (≥ ${min})`);
      for (const n of new Set(numbersIn(body))) if (!masterNums.includes(n)) warn('presentation/SCRIPT.md', line, `slide ${s[3]}: number ${n} is not in the master`);
    });
  }
}

// ---------------------------------------------------------------- report
for (const w of warnings) console.log(`⚠ ${w}`);
for (const e of errors) console.log(`✘ ${e}`);
console.log(`\ncheck${FINAL ? ' --final' : ''}: ${errors.length} error(s), ${warnings.length} warning(s)`);
process.exit(errors.length ? 1 : 0);
