# Cinefly EffectCraft Plugin Port — planning workspace

| Read | What |
|---|---|
| [`EFFECTCRAFT-PLUGIN-PORT-PLAN.html`](./EFFECTCRAFT-PLUGIN-PORT-PLAN.html) | **The plan, rendered**: themed HTML with the draw.io diagrams and mockup screenshots inline. |
| [`EFFECTCRAFT-PLUGIN-PORT-SCOPING.md`](./EFFECTCRAFT-PLUGIN-PORT-SCOPING.md) | **The single source of truth.** Part A business view · Part B technical design · Part C delivery · appendices. |
| [`RESUME.md`](./RESUME.md) | Where the planning work stands and what comes next (generated from `PLAN-STATE.json`). |

## Layout

| Path | What |
|---|---|
| `evidence/NN-*.md` | Research findings, one topic per file, every fact with its source anchor. `00` is the brief. |
| `analysis/` | Read-only snapshots of deployed-only sources (configs, definitions, templates). No user data. |
| `diagrams/src/*.mjs` → `diagrams/drawio/*.drawio + .svg` | Diagram definitions (built with the kit) and their exports. |
| `mockups/capture/*.mjs` → `mockups/*.html` → `mockups/shots/*.png` | Capture scripts that render the real products at their pins, the galleries they generate, and the screenshots the plan embeds. |
| `review/` | What the reviews render and report: diagrams, HTML tiles, mockups, anchors (every code anchor with its excerpt), work in flight. |
| generated views (`DECISIONS.md`, `modules/`, `tasks/`, …) | Written from tagged master sections by `build/build-plan.mjs`. Never edit them. |
| `build/` | The scoping-plan kit (installed copies; see `build/KIT-INSTALLED.md`). Never edit it here. |
| `plan.config.mjs` | This plan's settings: names, mockup galleries, generators. |

## Build

```bash
npm install                      # once: marked, puppeteer, mermaid-cli
node build/build-diagrams.mjs    # diagrams/src → .drawio + .svg (needs draw.io Desktop)
node build/build-plan.mjs        # views + mockup shots + the rendered HTML
node build/review.mjs all        # PNGs + rendered checks in review/ — then look at them
node build/review.mjs anchors    # every code anchor resolved at its pin, with its excerpt
node build/inflight.mjs --gh     # branches, PRs and issues moving on the plan's surfaces
node build/check.mjs --final     # the writing and consistency rules
```
