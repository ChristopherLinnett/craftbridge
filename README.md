# CraftBridge

CraftBridge puts EffectCraft's motion-graphics engine inside iOS and Android apps. A Swift package and a Kotlin library expose everything the EffectCraft command line does as device APIs: open and save projects, run any engine command, inspect and edit properties, preview into the app's own view, render frames and export video. The GPU compositor draws through Metal on iOS and Vulkan on Android, and the Swift and Kotlin APIs are generated from the engine's own command list.

CraftBridge is an independent SDK, based on EffectCraft by the ArtCraft team (https://github.com/storytold/effectcraft).

**Status:** pre-release. Nothing is published yet.

## Layout

| Folder | What |
|---|---|
| `rust/` | The Cargo workspace: the universal core (`rust/core/`) and the EffectCraft plugin (`rust/effects/`) |
| `ios/sdk/` | The Swift package: products `CraftBridge` and `CraftBridgeEffects` |
| `android/sdk/` | The Gradle build: `io.github.christopherlinnett.craftbridge:core` and `:effects` |
| `conformance/` | The scenarios every platform runs against the EffectCraft command line |
| `tools/` | `doctor.sh` (prerequisites), `ci.sh` (the gate), `env.sh` (the build environment), `release.sh` (stages a release, publishing nothing), and the scripts that build the native library, move the upstream pin, run the device tests and run the conformance oracle |
| `plan/` | The plan of record |
| `.github/` | The hosted CI workflow: the full gate, on every pull request to `main` and every push to it |

## Working on it

```bash
tools/doctor.sh          # checks every prerequisite and prints the fix for each gap
tools/ci.sh <stage>…     # the gate's stages for what a change touches: formatters, linters, tests, the rule checks
```

Hosted CI runs every stage of `tools/ci.sh` on each pull request to `main` and each push to it; locally a change runs only the stages for what it touches (`AGENTS.md` §12). The rules every change follows are in [`AGENTS.md`](AGENTS.md). The design, including the layout and the gate, is the plan of record in [`plan/`](plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md).

## Licence

MIT OR Apache-2.0, at your option ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)). See [`NOTICE`](NOTICE).
