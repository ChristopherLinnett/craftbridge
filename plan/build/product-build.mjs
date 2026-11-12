// Build a product at its pinned SHA into a static folder for the capture harness (build/capture.mjs),
// without touching the repository: the source is exported with `git archive`, never checked out.
//
//   node build/product-build.mjs <repo>                       export + report what the build needs
//   node build/product-build.mjs <repo> --install             … and install dependencies (npm ci / pnpm / yarn)
//   node build/product-build.mjs <repo> --build "npx vite build" --env VITE_API_URL=https://api.mock.local …
//                                                             … and run the build with those env values
//   options: --out <dir> (default <tmp>/scoping-builds) · --env-file <file> (KEY=VALUE lines)
//
// The report lists every environment variable the source reads (import.meta.env / process.env, with the
// first file:line), whether .env.example and the CI workflows set it, and a placeholder suggestion.
// Point API base URLs at a host the capture fixtures answer (e.g. https://api.mock.local); use public or
// test identifiers only — never a secret. Output: the build folder to pass to capture({ serve: { dir } }).
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execSync, spawnSync } from 'node:child_process';
import { loadConfig } from './lib/common.mjs';
import { repoList } from './lib/repos.mjs';

const cfg = await loadConfig();
const argv = process.argv.slice(2);
const flag = (k) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : null; };
const VALUE_FLAGS = ['--out', '--build', '--env', '--env-file'];
const key = argv.find((a, i) => !a.startsWith('--') && !VALUE_FLAGS.includes(argv[i - 1]));
const repo = repoList(cfg).find((r) => r.key === key || r.aliases.includes(key));
if (!repo) { console.error(`✘ usage: node build/product-build.mjs <repo> — one of: ${Object.keys(cfg.repos || {}).join(', ') || '(plan.config.mjs has no repos)'}`); process.exit(1); }
if (repo.problem) { console.error(`✘ ${repo.problem}`); process.exit(1); }

const out = path.join(flag('out') || path.join(os.tmpdir(), 'scoping-builds'), `${repo.key}@${repo.sha}`);
if (!fs.existsSync(path.join(out, 'package.json')) && !fs.existsSync(path.join(out, '.scoping-exported'))) {
  fs.mkdirSync(out, { recursive: true });
  execSync(`git -C "${repo.dir}" archive ${repo.full} | tar -x -C "${out}"`, { stdio: 'inherit', shell: '/bin/sh' });
  fs.writeFileSync(path.join(out, '.scoping-exported'), `${repo.key}@${repo.full}\n`);
  console.log(`✔ exported ${repo.key}@${repo.sha} → ${out}`);
} else console.log(`✔ ${out} already holds ${repo.key}@${repo.sha}`);

// ---------------------------------------------------------------- what the build needs
const pkg = fs.existsSync(path.join(out, 'package.json')) ? JSON.parse(fs.readFileSync(path.join(out, 'package.json'), 'utf8')) : {};
const deps = { ...pkg.dependencies, ...pkg.devDependencies };
const framework = deps.astro ? 'astro' : deps.next ? 'next' : deps.vite ? 'vite' : deps['react-scripts'] ? 'create-react-app' : deps['@angular/core'] ? 'angular' : 'unknown';
const read = (f) => (fs.existsSync(path.join(out, f)) ? fs.readFileSync(path.join(out, f), 'utf8') : '');
const viteCfg = ['vite.config.ts', 'vite.config.js', 'vite.config.mjs'].map(read).find(Boolean) || '';
const outDir = (viteCfg.match(/outDir:\s*['"]([^'"]+)['"]/) || [])[1] || { astro: 'dist', next: 'out', vite: 'dist', 'create-react-app': 'build', angular: 'dist' }[framework] || 'dist';
console.log(`\nframework: ${framework} · build output: ${outDir}/ · scripts: ${Object.entries(pkg.scripts || {}).filter(([k]) => /build|prebuild|postbuild/.test(k)).map(([k, v]) => `${k} = "${v}"`).join(' · ') || '(none)'}`);

const vars = new Map();
const walk = (d) => {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    if (['node_modules', '.git', 'dist', 'build', '.next', 'out', 'coverage'].includes(e.name)) continue;
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p);
    else if (/\.(m?[jt]sx?|astro|vue|svelte)$/.test(e.name) && fs.statSync(p).size < 2e6) {
      fs.readFileSync(p, 'utf8').split('\n').forEach((l, i) => {
        for (const m of l.matchAll(/(?:import\.meta\.env|process\.env)\.([A-Z][A-Z0-9_]+)/g)) if (!vars.has(m[1])) vars.set(m[1], `${path.relative(out, p)}:${i + 1}`);
      });
    }
  }
};
walk(out);
const example = ['.env.example', '.env.sample', '.env.template', '.env.development'].map(read).join('\n');
const ci = fs.existsSync(path.join(out, '.github/workflows')) ? fs.readdirSync(path.join(out, '.github/workflows')).map((f) => read(`.github/workflows/${f}`)).join('\n') : '';
const suggest = (k) => (/STRIPE.*(PUBLISHABLE|PUBLIC|KEY)/.test(k) ? 'pk_test_placeholder'
  : /(URL|ENDPOINT|HOST|DOMAIN|BASE|ORIGIN)/.test(k) ? 'https://api.mock.local (or the public site URL)'
    : /REGION/.test(k) ? 'the real region' : /(SECRET|PASSWORD|PRIVATE|TOKEN)/.test(k) ? '— leave unset (never a secret)' : 'fixture');
if (vars.size) {
  console.log(`\n${vars.size} environment variable(s) read by the source:\n`);
  console.log('| Variable | First read at | .env.example | CI | Placeholder |\n|---|---|---|---|---|');
  for (const [k, where] of [...vars].sort()) console.log(`| ${k} | ${where} | ${new RegExp(`^\\s*${k}=`, 'm').test(example) ? 'yes' : ''} | ${ci.includes(k) ? 'yes' : ''} | ${suggest(k)} |`);
}

// ---------------------------------------------------------------- install + build
const lock = fs.existsSync(path.join(out, 'pnpm-lock.yaml')) ? ['pnpm', ['install', '--frozen-lockfile']] : fs.existsSync(path.join(out, 'yarn.lock')) ? ['yarn', ['install', '--frozen-lockfile']] : ['npm', ['ci', '--no-audit', '--no-fund']];
if (argv.includes('--install') || flag('build')) {
  if (!fs.existsSync(path.join(out, 'node_modules'))) {
    console.log(`\n$ ${lock[0]} ${lock[1].join(' ')}`);
    const r = spawnSync(lock[0], lock[1], { cwd: out, stdio: 'inherit' });
    if (r.status) process.exit(r.status);
  } else console.log('\n✔ dependencies already installed');
}
if (flag('build')) {
  const env = { ...process.env };
  const envFile = flag('env-file');
  if (envFile) for (const l of fs.readFileSync(envFile, 'utf8').split('\n')) { const m = l.match(/^\s*([A-Z][A-Z0-9_]*)=(.*)$/); if (m) env[m[1]] = m[2]; }
  argv.forEach((a, i) => { if (a === '--env' && argv[i + 1]) { const [k, ...v] = argv[i + 1].split('='); env[k] = v.join('='); } });
  console.log(`\n$ ${flag('build')}`);
  const r = spawnSync(flag('build'), { cwd: out, stdio: 'inherit', shell: true, env });
  if (r.status) process.exit(r.status);
  const dir = path.join(out, outDir);
  if (!fs.existsSync(dir)) { console.error(`✘ the build finished but ${dir} does not exist — check the output folder (vite outDir / framework default)`); process.exit(1); }
  const count = (d) => fs.readdirSync(d, { withFileTypes: true }).reduce((n, e) => n + (e.isDirectory() ? count(path.join(d, e.name)) : 1), 0);
  console.log(`\n✔ built: ${dir} (${count(dir)} files) — capture({ serve: { origin: '<the product's real origin>', dir: '${dir}'${['vite', 'create-react-app', 'angular'].includes(framework) ? ', spa: true' : ''} } })`);
  console.log('  Record the exact command and env names (not secret values) in mockups/capture/README.md so the frames can be rebuilt.');
}
