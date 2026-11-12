// scoping-plan kit — read-only access to the repositories a plan is pinned to.
// Pins live in plan.config.mjs → repos: { '<repo>': { path, sha, base = 'origin/main', watch: [paths] } }
// (paths relative to the plan folder). Nothing here fetches, checks out or writes: it reads objects
// at the pinned SHA and compares refs the last `git fetch` brought in.
// Installed into <plan>/build/lib/. Change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { ROOT } from './common.mjs';

export function git(dir, args, { allowFail = false } = {}) {
  try {
    return execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
  } catch (e) {
    if (allowFail) return null;
    throw new Error(`git -C ${dir} ${args.join(' ')}: ${(e.stderr || e.message).toString().trim().split('\n')[0]}`);
  }
}

// The configured repositories, resolved. Missing folders are reported, not thrown.
export function repoList(cfg) {
  return Object.entries(cfg.repos || {}).map(([key, r]) => {
    const dir = path.resolve(ROOT, r.path || key);
    const ok = fs.existsSync(path.join(dir, '.git')) || fs.existsSync(path.join(dir, 'HEAD'));
    const full = ok ? git(dir, ['rev-parse', '--verify', '--quiet', `${r.sha}^{commit}`], { allowFail: true })?.trim() : null;
    return { key, dir, sha: r.sha, base: r.base || 'origin/main', watch: r.watch || [], aliases: r.aliases || [], ok, full, problem: !ok ? `no git repository at ${dir}` : !full ? `pinned SHA ${r.sha} is not in ${dir} — git fetch, or fix the pin` : null };
  });
}

const lsCache = new Map();
export function lsFiles(repo) {
  if (!lsCache.has(repo.key)) lsCache.set(repo.key, git(repo.dir, ['ls-tree', '-r', '--name-only', repo.full]).split('\n').filter(Boolean));
  return lsCache.get(repo.key);
}
const showCache = new Map();
export function showAt(repo, file) {
  const k = `${repo.key}:${file}`;
  if (!showCache.has(k)) showCache.set(k, git(repo.dir, ['show', `${repo.full}:${file}`], { allowFail: true }));
  return showCache.get(k);
}

// What moved since the pin: commits on the base branch, pinned files changed there, and every
// recently active remote branch whose diff touches the watched paths or the anchored files.
// This is how work in flight (a branch rebuilding the same page, an unmerged fix) is found
// before the plan is built on a stale picture.
export function movingGround(repo, files = [], { days = 45, maxBranches = 60 } = {}) {
  const out = { key: repo.key, base: repo.base, sha: repo.sha };
  const fetchHead = git(repo.dir, ['rev-parse', '--git-path', 'FETCH_HEAD'], { allowFail: true })?.trim();
  const fh = fetchHead && path.resolve(repo.dir, fetchHead);
  out.lastFetch = fh && fs.existsSync(fh) ? new Date(fs.statSync(fh).mtimeMs).toISOString().slice(0, 16).replace('T', ' ') : 'never';
  const baseSha = git(repo.dir, ['rev-parse', '--verify', '--quiet', repo.base], { allowFail: true })?.trim();
  if (!baseSha) { out.problem = `base ${repo.base} not found`; return out; }
  out.baseSha = baseSha.slice(0, 8);
  out.onBase = git(repo.dir, ['merge-base', '--is-ancestor', repo.full, baseSha], { allowFail: true }) !== null;
  const from = out.onBase ? repo.full : git(repo.dir, ['merge-base', repo.full, baseSha]).trim();
  out.behind = +git(repo.dir, ['rev-list', '--count', `${from}..${baseSha}`]).trim();
  if (!out.onBase) out.containedIn = git(repo.dir, ['branch', '-r', '--contains', repo.full]).split('\n').map((s) => s.trim()).filter(Boolean).slice(0, 8);
  const watched = [...new Set([...repo.watch, ...files])];
  const touches = (changed) => changed.filter((f) => watched.some((w) => f === w || f.startsWith(w.endsWith('/') ? w : `${w}/`)));
  out.changedOnBase = watched.length && out.behind ? touches(git(repo.dir, ['diff', '--name-only', from, baseSha]).split('\n').filter(Boolean)) : [];
  const since = Date.now() - days * 86400000;
  const refs = git(repo.dir, ['for-each-ref', '--sort=-committerdate', '--format=%(refname:short)\t%(committerdate:iso-strict)\t%(authorname)\t%(subject)', 'refs/remotes'])
    .split('\n').filter(Boolean).map((l) => { const [name, date, author, subject] = l.split('\t'); return { name, date, author, subject }; })
    .filter((r) => !/\/HEAD$/.test(r.name) && r.name !== repo.base && Date.parse(r.date) >= since).slice(0, maxBranches);
  out.activeBranches = refs.length;
  out.branches = [];
  for (const r of refs) {
    const mb = git(repo.dir, ['merge-base', baseSha, r.name], { allowFail: true })?.trim();
    if (!mb) continue;
    const ahead = +git(repo.dir, ['rev-list', '--count', `${mb}..${r.name}`]).trim();
    if (!ahead) continue;
    const changed = git(repo.dir, ['diff', '--name-only', mb, r.name]).split('\n').filter(Boolean);
    const hit = watched.length ? touches(changed) : [];
    const containsPin = git(repo.dir, ['merge-base', '--is-ancestor', repo.full, r.name], { allowFail: true }) !== null;
    if (hit.length || (containsPin && !out.onBase)) out.branches.push({ ...r, date: r.date.slice(0, 10), ahead, files: changed.length, hit, containsPin });
  }
  return out;
}

// owner/repo from the origin remote, for gh.
export function ghSlug(repo) {
  const url = git(repo.dir, ['remote', 'get-url', 'origin'], { allowFail: true })?.trim() || '';
  const m = url.match(/github\.com[:/]([^/]+\/[^/.]+)(?:\.git)?$/);
  return m ? m[1] : null;
}

export function movingGroundMd(mg) {
  const lines = [`### ${mg.key}`, ''];
  if (mg.problem) return [...lines, `- ✘ ${mg.problem}`, ''];
  lines.push(`- pinned \`${mg.sha}\`${mg.onBase ? ` on ${mg.base}` : ` — **not on ${mg.base}** (it is on ${(mg.containedIn || []).join(', ') || 'no remote branch'})`}; ${mg.base} is \`${mg.baseSha}\`, **${mg.behind} commit(s) ahead** of the pin; last fetch ${mg.lastFetch}`);
  if (mg.changedOnBase.length) lines.push(`- ⚠ watched / anchored files changed on ${mg.base} since the pin: ${mg.changedOnBase.map((f) => `\`${f}\``).join(', ')} — re-read them and move the pin`);
  if (mg.branches.length) {
    lines.push(`- ⚠ ${mg.branches.length} of ${mg.activeBranches} recently active remote branch(es) touch watched / anchored files (or carry the pin):`, '', '| Branch | Last commit | Author | Ahead | Watched files touched |', '|---|---|---|---|---|');
    for (const b of mg.branches) lines.push(`| \`${b.name}\` | ${b.date} · ${b.subject.replace(/\|/g, '/').slice(0, 70)} | ${b.author} | ${b.ahead} | ${b.hit.length ? b.hit.slice(0, 6).map((f) => `\`${f}\``).join(', ') + (b.hit.length > 6 ? ` +${b.hit.length - 6}` : '') : b.containsPin ? '(contains the pin)' : ''} |`);
    lines.push('', '  Read each: `git log --oneline <base>..<branch>` and `git diff <base>...<branch> -- <file>`. The plan stays the source of truth: record what each branch does, and state what the build keeps, replaces or leaves alone.');
  } else lines.push(`- no recently active remote branch touches the watched / anchored files (${mg.activeBranches} active in the window)`);
  return [...lines, ''];
}
