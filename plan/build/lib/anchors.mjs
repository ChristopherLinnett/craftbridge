// scoping-plan kit — find every code anchor in the plan (`path/file.ts:120-138`) and resolve it
// against the pinned repositories (plan.config.mjs → repos) and the deployed snapshots in analysis/.
// For each anchor it reports: the file it resolves to, whether the lines exist, the excerpt, and
// whether the identifiers the same sentence names actually appear there. A reader (or a model)
// then only has to judge whether the excerpt says what the sentence claims.
// Installed into <plan>/build/lib/. Change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT } from './common.mjs';
import { repoList, lsFiles, showAt, git } from './repos.mjs';

const CODE_EXT = /\.(?:[a-z]{1,6}|j2)$/i;
const MAIN_FILES = ['lambda_function.py', 'index.mjs', 'index.js', 'index.ts', 'handler.py', 'app.py', 'main.py', 'handler.js'];

// ---------------------------------------------------------------- parsing
function parseRanges(s) {
  return s.split(/,\s*/).map((r) => { const [a, b] = r.split('-').map(Number); return [a, b || a]; });
}
// Identifiers a sentence names in code spans: camelCase, snake_case, PascalCase.dotted, calls, "literals".
function identifiersIn(text, skip) {
  const ids = new Set();
  for (const m of text.matchAll(/`([^`\n]+)`/g)) {
    if (skip.has(m[1])) continue;
    for (const lit of m[1].matchAll(/"([\w.:/-]{4,})"/g)) ids.add(lit[1]);
    for (const w of m[1].matchAll(/[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*/g)) {
      const t = w[0];
      if (t.length < 4) continue;
      if (/[a-z][A-Z]/.test(t) || /_/.test(t) || /^[A-Z][a-z]+[A-Z]/.test(t) || new RegExp(`${t.replace(/[$.]/g, '\\$&')}\\(`).test(m[1])) ids.add(t.split('.').pop());
    }
  }
  return [...ids].filter((t) => t.length >= 4);
}

export function findAnchors(file, text) {
  const out = [];
  let inCode = false;
  text.split('\n').forEach((line, i) => {
    if (/^```/.test(line)) { inCode = !inCode; return; }
    if (inCode || /^\s*<!--/.test(line)) return;
    // Scope identifiers to the sentence (inside a table cell) that holds the anchor.
    const cells = (line.startsWith('|') ? line.split(/(?<!\\)\|/) : [line]).flatMap((c) => c.split(/(?<=[.;!?])\s+(?=[A-Z*(])/));
    let prev = null;
    for (const [ci, cell] of cells.entries()) {
      const spans = [...cell.matchAll(/`([^`\n]+)`/g)].map((m) => m[1]);
      const anchorSpans = new Set();
      const found = [];
      for (const raw of spans) {
        let c = raw.trim();
        let repoHint = null;
        const hint = c.match(/^([\w.-]+)(?:@[0-9a-f]{7,40})?:\s+(\S.*)$/);
        if (hint) { repoHint = hint[1]; c = hint[2]; }
        const cont = c.match(/^:(\d+(?:-\d+)?(?:,\s?\d+(?:-\d+)?)*)$/);
        if (cont && prev) { found.push({ ...prev, ranges: parseRanges(cont[1]), raw }); anchorSpans.add(raw); continue; }
        const m = c.match(/^([\w./@+-]+?):(\d+(?:-\d+)?(?:,\s?\d+(?:-\d+)?)*)$/);
        if (!m || /^\d+$/.test(m[1]) || /:\/\//.test(c)) continue;
        const target = m[1];
        const classMethod = /^[A-Z]\w+\.[a-z]\w+$/.test(target) && !CODE_EXT.test(target.split('.').pop().length <= 4 ? target : '');
        const looks = target.includes('/') || CODE_EXT.test(target) || /^[\w]+-[\w-]+$/.test(target) || classMethod;
        if (!looks) continue;
        const a = { file, line: i + 1, raw, target, repoHint, ranges: parseRanges(m[2]), classMethod };
        prev = a; found.push(a); anchorSpans.add(raw);
      }
      for (const a of found) out.push({ ...a, group: `${file}:${i + 1}:${ci}`, context: cell.trim().slice(0, 400), ids: identifiersIn(cell, anchorSpans) });
    }
  });
  return out;
}

// ---------------------------------------------------------------- resolution
let deployedIndex = null;
function deployedFiles(cfg) {
  if (deployedIndex) return deployedIndex;
  const roots = (cfg.deployed || ['analysis']).map((d) => path.join(ROOT, d)).filter((d) => fs.existsSync(d));
  const files = [];
  const dirs = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (e.name === 'node_modules' || e.name === '.git' || e.name.startsWith('.')) continue;
      const p = path.join(d, e.name);
      if (e.isDirectory()) { dirs.push(path.relative(ROOT, p)); if (files.length < 60000) walk(p); } else if (fs.statSync(p).size < 5e6) files.push(path.relative(ROOT, p));
    }
  };
  roots.forEach(walk);
  deployedIndex = { files, dirs };
  return deployedIndex;
}

function candidates(a, cfg, repos) {
  const c = [];
  const pool = a.repoHint ? repos.filter((r) => r.key === a.repoHint || r.aliases.includes(a.repoHint)) : repos;
  const match = (p) => p === a.target || p.endsWith(`/${a.target}`);
  for (const r of pool) {
    if (!r.full) continue;
    const files = lsFiles(r);
    let hits = files.filter(match);
    if (!hits.length && a.classMethod) {
      const cls = a.target.split('.')[0];
      hits = files.filter((p) => path.basename(p).startsWith(`${cls}.`));
    }
    for (const h of hits) c.push({ kind: 'repo', repo: r, path: h, read: () => showAt(r, h) });
  }
  if (!a.repoHint || !c.length) {
    const { files, dirs } = deployedFiles(cfg);
    for (const f of files.filter(match)) c.push({ kind: 'deployed', path: f, read: () => fs.readFileSync(path.join(ROOT, f), 'utf8') });
    if (!c.length) {
      for (const d of dirs.filter((x) => path.basename(x) === a.target)) {
        const inside = fs.readdirSync(path.join(ROOT, d));
        const main = MAIN_FILES.find((f) => inside.includes(f)) || (inside.filter((f) => /\.(py|js|mjs|ts)$/.test(f)).length === 1 ? inside.find((f) => /\.(py|js|mjs|ts)$/.test(f)) : null);
        if (main) c.push({ kind: 'deployed', path: path.join(d, main), read: () => fs.readFileSync(path.join(ROOT, d, main), 'utf8') });
      }
    }
  }
  return c;
}

// Context words on the line that point at one repo (its key, an alias, or its pinned SHA).
function preferByContext(c, a, repos) {
  if (c.length < 2) return c;
  const ctx = a.context.toLowerCase();
  const scored = c.map((x) => {
    if (x.kind !== 'repo') return { x, s: /deployed|lambda|analysis/.test(ctx) ? 2 : 0 };
    const r = x.repo;
    const words = [r.key, ...r.aliases, r.sha, ...r.key.split(/[-_]/).filter((w) => w.length > 3 && !['cinefly', 'server', 'backend'].includes(w))];
    return { x, s: words.some((w) => ctx.includes(String(w).toLowerCase())) ? 3 : 0 };
  });
  const best = Math.max(...scored.map((s) => s.s));
  return best ? scored.filter((s) => s.s === best).map((s) => s.x) : c;
}

// Does the identifier exist anywhere at the pin? A name that exists nowhere is new code the plan
// proposes, so it is not expected at the anchor.
const grepCache = new Map();
function existsAtPin(x, id) {
  if (x.kind !== 'repo') return true;
  const k = `${x.repo.key}:${id}`;
  if (!grepCache.has(k)) grepCache.set(k, git(x.repo.dir, ['grep', '-q', '-F', id, x.repo.full, '--'], { allowFail: true }) !== null);
  return grepCache.get(k);
}

export function resolveAnchors(cfg, anchors) {
  const repos = repoList(cfg);
  const res = [];
  for (const a of anchors) {
    let c = candidates(a, cfg, repos);
    c = preferByContext(c, a, repos);
    // Identical copies (a shared util vendored into several lambdas) count as one.
    const texts = c.map((x) => ({ x, t: x.read() })).filter((y) => y.t != null);
    const uniq = [];
    for (const y of texts) if (!uniq.some((u) => u.t === y.t)) uniq.push(y);
    const r = { ...a, status: 'ok', notes: [], candidates: texts.map((y) => (y.x.kind === 'repo' ? `${y.x.repo.key}@${y.x.repo.sha}:${y.x.path}` : y.x.path)) };
    if (a.repoHint && !repos.some((x) => x.key === a.repoHint || x.aliases.includes(a.repoHint))) r.notes.push(`repo "${a.repoHint}" is not in plan.config.mjs repos`);
    if (!uniq.length) { r.status = 'missing'; res.push(r); continue; }
    if (uniq.length > 1) { r.status = 'ambiguous'; r.notes.push(`matches ${uniq.length} different files — name the repo or a longer path`); }
    const { t } = uniq[0];
    const lines = t.split('\n');
    const max = Math.max(...a.ranges.map((x) => x[1]));
    r.resolved = r.candidates[0] + (texts.length > uniq.length ? ` (+${texts.length - 1} identical cop${texts.length - 1 === 1 ? 'y' : 'ies'})` : '');
    r.fileLines = lines.length;
    if (max > lines.length) { r.status = 'range'; r.notes.push(`line ${max} is past the end of the file (${lines.length} lines)`); }
    const [lo, hi] = [Math.min(...a.ranges.map((x) => x[0])), max];
    r.excerpt = lines.slice(lo - 1, Math.min(hi, lo + 29)).map((l, k) => `${String(lo + k).padStart(5)}  ${l}`).join('\n');
    r.window = lines.slice(Math.max(0, lo - 4), hi + 3).join('\n');
    r.ids = a.ids.filter((id) => existsAtPin(uniq[0].x, id));
    r.newNames = a.ids.filter((id) => !r.ids.includes(id));
    r.idsFound = r.ids.filter((id) => r.window.includes(id));
    res.push(r);
  }
  // A sentence citing several anchors is satisfied when its identifiers appear at any of them.
  const groups = new Map();
  for (const r of res) (groups.get(r.group) || groups.set(r.group, []).get(r.group)).push(r);
  for (const g of groups.values()) {
    const ids = [...new Set(g.flatMap((r) => r.ids || []))];
    const found = ids.filter((id) => g.some((r) => r.window?.includes(id)));
    for (const r of g) {
      if (r.status !== 'ok' || !ids.length) continue;
      if (!found.length) { r.status = 'weak'; r.notes.push(`none of ${ids.slice(0, 5).join(', ')} appear at ${g.length > 1 ? 'any of this sentence\'s anchors' : 'these lines'}`); }
      else if (!r.idsFound.length && g.length > 1) r.notes.push('no identifier from the sentence at these lines (the sentence\'s other anchors carry them)');
    }
  }
  return { repos, anchors: res };
}

export function anchorsReportMd(result) {
  const icon = { ok: '✔', weak: '⚠', ambiguous: '⚠', range: '✘', missing: '✘' };
  const lines = [];
  for (const r of result.anchors) {
    lines.push(`### ${icon[r.status]} \`${r.raw}\` — ${r.file}:${r.line}`, '');
    lines.push(`> ${r.context.replace(/\n/g, ' ')}`, '');
    if (r.resolved) lines.push(`- resolves to \`${r.resolved}\`${r.candidates.length > 1 ? ` · candidates: ${r.candidates.map((c) => `\`${c}\``).join(', ')}` : ''}`);
    else lines.push('- ✘ not found in any pinned repo or in analysis/');
    if (r.ids?.length) lines.push(`- identifiers named: ${r.ids.map((id) => (r.idsFound?.includes(id) ? `\`${id}\` ✔` : `\`${id}\` ✘`)).join(' · ')}`);
    if (r.newNames?.length) lines.push(`- new names (not in the repo at the pin — proposed code): ${r.newNames.map((id) => `\`${id}\``).join(', ')}`);
    for (const n of r.notes) lines.push(`- ${icon[r.status] === '✔' ? 'ℹ' : icon[r.status]} ${n}`);
    if (r.excerpt) lines.push('', '```', r.excerpt, '```');
    lines.push('');
  }
  return lines;
}
