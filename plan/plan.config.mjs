// EffectCraft Plugin Port plan — configuration for the scoping-plan kit (build/*.mjs, installed by the skill).
// Plan-specific settings and code live here, never in build/ (build/ is replaced on every install).
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// The pinned repositories sit beside the repository's main checkout, but a git worktree of it lives
// elsewhere (AGENTS.md §12: ../craftbridge-worktrees/<branch>), where paths relative to its own plan
// folder would find nothing. So each repo path is written as seen from the main checkout's plan folder,
// which git names from any worktree (its common git dir is the main checkout's .git), and handed to the
// kit relative to this folder, as it expects. Outside git, this folder is the main checkout's.
const here = path.dirname(fileURLToPath(import.meta.url));
const mainPlan = (() => {
  try {
    const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], { cwd: here, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    return path.join(path.dirname(common), 'plan');
  } catch {
    return here;
  }
})();
const besideMain = (relative) => path.relative(here, path.resolve(mainPlan, relative)) || '.';

export default {
  name: 'EffectCraft Plugin Port',
  master: 'EFFECTCRAFT-PLUGIN-PORT-SCOPING.md',
  html: 'EFFECTCRAFT-PLUGIN-PORT-PLAN.html',
  htmlTitle: 'EffectCraft Plugin Port: CraftBridge for iOS and Android — Plan of Record',
  date: '2026-11-12',                 // shown as "Plan of record · <date>"; update at sign-off
  footerNote: '',                   // e.g. 'verified against origin/main and staging, <date>'

  // One entry per mockup gallery (mockups/<surface>.html). Frames are <figure class="frame" id="<prefix><n>"
  // data-source="…">; a capture script (mockups/capture/<surface>.mjs) generates the gallery.
  //   { prefix: 'm', kind: 'mobile', file: 'mockups/mobile.html', deviceScale: 2 }
  //   { prefix: 'p', kind: 'portal', file: 'mockups/portal.html', waitReady: true }   // page sets <html data-ready="1">
  // kind 'mobile' renders phone-sized in the HTML; anything else renders full width.
  mockups: [],

  // The repositories the plan is pinned to — the single source of the pins (the master header states
  // them; check.mjs fails if it doesn't). Paths are relative to the main checkout's plan folder (above). watch: the paths the
  // feature touches, for build/inflight.mjs (work in flight) and build/review.mjs anchors.
  //   'app-web': { path: '../app-web', sha: '1a2b3c4d', base: 'origin/main', aliases: ['web'], watch: ['src/features/checkout'] },
  repos: {
    effectcraft: { path: besideMain('../../effectcraft/repository'), sha: '10376058', base: 'origin/main', aliases: ['EffectCraft', 'storytold/effectcraft'],
      watch: ['apps/effectcraft-cli', 'crates/automation', 'crates/host', 'crates/engine/src/cmd.rs', 'crates/engine/src/cmd', 'crates/engine/src/lib.rs',
              'crates/engine/src/services.rs', 'crates/engine/src/render_queue.rs', 'crates/gpu/src/context.rs', 'crates/gpu/src/lib.rs',
              'crates/render/src/footage.rs', 'crates/export/src', 'crates/media/src', 'crates/text/src/fonts.rs', 'crates/host/src/app_dirs.rs',
              'crates/engine/src/system/sysinfo.rs', 'Cargo.toml', 'xtask/src/main.rs'] },
    figcraft: { path: besideMain('../../figcraft/repository'), sha: 'a4251ca8', base: 'origin/main', aliases: ['FigCraft', 'storytold/figcraft'], watch: ['crates/engine/src/cmd.rs', 'crates/engine/src/lib.rs'] },
    'craft-libs': { path: besideMain('../../../../.cargo/git/checkouts/craft-libs-d830b991a08c6c3d/e7c3dda'), sha: 'e7c3dda', base: 'e7c3dda', aliases: ['craft-libs', 'storytold/craft-libs'], watch: ['crates/web-api', 'crates/font-catalog', 'crates/video/src/h264/encode'] },
    filmcraft: { path: besideMain('../../filmcraft/repository'), sha: 'd5ead4ba', base: 'origin/main', aliases: ['FilmCraft', 'storytold/filmcraft'], watch: ['crates/codecs', 'crates/media'] },
  },
  // Words for the in-flight sweep (branch names, PR and issue titles): node build/inflight.mjs --gh
  keywords: ['mobile', 'ios', 'android', 'uniffi', 'ffi', 'swift', 'kotlin', 'sdk', 'staticlib', 'cdylib', 'xcframework', 'aar', 'metal', 'vulkan'],
  // Folders code anchors may resolve into besides the repos (deployed-only snapshots).
  deployed: ['analysis'],

  // Generated tables: <!-- @gen <name> <args> --> … <!-- @end-gen --> in the master is replaced by
  // generators[name](args, { read, root }) on every build (e.g. a table rendered from a JSON fixture).
  generators: {},

  // Shareable bundle (build/bundle.mjs → <slug>-plan.zip). analysis/ is never bundled; opt in to
  // evidence/ only if the research notes are fit for everyone the bundle is shared with.
  bundle: { include: [] },
};
