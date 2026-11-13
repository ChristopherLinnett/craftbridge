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
| `tools/` | `doctor.sh` (prerequisites), `ci.sh` (the gate), `env.sh` (the build environment); the build and release scripts arrive with the waves that need them |
| `plan/` | The plan of record |

## Working on it

```bash
tools/doctor.sh      # checks every prerequisite and prints the fix for each gap
tools/ci.sh          # the gate: formatters, linters, tests and the rule checks
```

The rules every change follows are in [`AGENTS.md`](AGENTS.md). The design, including the layout and the gate, is the plan of record in [`plan/`](plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md).

## Licence

MIT OR Apache-2.0, at your option ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)). See [`NOTICE`](NOTICE).
