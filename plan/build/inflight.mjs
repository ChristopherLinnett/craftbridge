// Work in flight on the plan's surfaces — run it during research, before trusting any pin, and again
// at verify. For every repo in plan.config.mjs → repos it reports (read-only, from the last fetch):
//   · how far the base branch has moved past the pin, and which watched / anchored files changed,
//   · every recently active remote branch that touches the watched paths or the anchored files
//     (a branch rebuilding the same page is the classic miss),
//   · with --gh: open and recent PRs and issues matching plan.config.mjs → keywords.
//   node build/inflight.mjs [--days 45] [--gh]        → review/inflight/REPORT.md
// Fetch first (git -C <repo> fetch --all --prune): this script never fetches or checks anything out.
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { ROOT, loadConfig } from './lib/common.mjs';
import { repoList, movingGround, movingGroundMd, ghSlug } from './lib/repos.mjs';
import { findAnchors, resolveAnchors } from './lib/anchors.mjs';

const cfg = await loadConfig();
const argv = process.argv.slice(2);
const days = +(argv[argv.indexOf('--days') + 1] || 0) || 45;
const useGh = argv.includes('--gh');
const repos = repoList(cfg);
if (!repos.length) {
  console.log('✘ plan.config.mjs has no repos — add one entry per repository the plan touches:\n  repos: { \'<repo>\': { path: \'../<repo>\', sha: \'<short sha>\', base: \'origin/main\', watch: [\'src/<feature paths>\'] } }');
  process.exit(1);
}
// Files the master already anchors, per repo: they are watched too.
const master = fs.existsSync(path.join(ROOT, cfg.master)) ? fs.readFileSync(path.join(ROOT, cfg.master), 'utf8') : '';
const anchored = {};
if (master) {
  for (const a of resolveAnchors(cfg, findAnchors(cfg.master, master)).anchors) {
    for (const c of a.candidates) { const m = c.match(/^([^@]+)@[^:]+:(.+)$/); if (m) (anchored[m[1]] ||= new Set()).add(m[2]); }
  }
}
const lines = ['# Work in flight', '', `Generated ${new Date().toISOString().slice(0, 16).replace('T', ' ')} by build/inflight.mjs (read-only; from each repo's last fetch). Window: ${days} days.`, '',
  'Every ⚠ below is a lead to read, not a verdict. The plan is the source of truth; in-flight work is evidence. Open each branch or PR, record what it does, and state in the plan what the build keeps, replaces or leaves alone (product choices it implies go to the user as questions).', ''];
let warnings = 0;
for (const r of repos) {
  if (r.problem) { lines.push(`### ${r.key}`, '', `- ✘ ${r.problem}`, ''); warnings++; continue; }
  const mg = movingGround(r, [...(anchored[r.key] || [])], { days });
  if (mg.changedOnBase?.length || mg.branches?.length || !mg.onBase) warnings++;
  lines.push(...movingGroundMd(mg));
  if (useGh) {
    const slug = ghSlug(r);
    const kws = cfg.keywords || [];
    if (!slug) lines.push('- (no GitHub origin — PRs and issues not checked)', '');
    else if (!kws.length) lines.push('- (plan.config.mjs has no keywords — PRs and issues not searched)', '');
    else {
      for (const kind of ['pr', 'issue']) {
        const seen = new Map();
        for (const kw of kws) {
          try {
            const out = execFileSync('gh', [kind, 'list', '-R', slug, '--state', 'all', '--search', kw, '--limit', '15', '--json', 'number,title,state,updatedAt'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
            for (const x of JSON.parse(out)) if (Date.parse(x.updatedAt) > Date.now() - days * 86400000) seen.set(x.number, x);
          } catch (e) { lines.push(`- gh ${kind} list failed: ${(e.stderr || e.message).toString().split('\n')[0]}`); break; }
        }
        if (seen.size) {
          lines.push(`- ${kind === 'pr' ? 'Pull requests' : 'Issues'} matching ${kws.map((k) => `"${k}"`).join(', ')} (updated in the window):`, '');
          for (const x of [...seen.values()].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))) lines.push(`  - ${slug}#${x.number} [${x.state}] ${x.title} (${x.updatedAt.slice(0, 10)})`);
          lines.push('');
          warnings++;
        }
      }
    }
  }
}
const dir = path.join(ROOT, 'review', 'inflight');
fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(dir, 'REPORT.md'), lines.join('\n') + '\n');
console.log(lines.filter((l) => /^###|✘|⚠/.test(l)).join('\n'));
console.log(`\ninflight: ${repos.length} repo(s), ${warnings} with leads to read → review/inflight/REPORT.md`);
