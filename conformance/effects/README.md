# EffectCraft conformance corpus

The scenarios (T1–T24 of the plan, §28) that every platform will run against the EffectCraft command line, which is the oracle, together with their fixtures and the running example's template recipe. The data here belongs to no platform: the desktop runner (`rust/effects/craftbridge-effects-conformance`), the Swift tests (`ios/sdk/Tests`) and the Android instrumented tests (`android/sdk/effects/src/androidTest`) will all read it, so a scenario is written once. The scenarios and fixtures arrive with the wave that first runs them; this folder is their home from the start.

`corpus.toml` holds the scenarios, and its header says how one is written: steps (one oracle invocation each) and what is checked after them, on the oracle and on the side under test, whose files a comparison holds against the oracle's. The desktop runner compares what the command line produces with what the corpus says it must, and for an encoded export applies D-12: the same frame count and size, and every decoded frame within 40 dB PSNR.

The oracle is the official EffectCraft command line at the pinned revision, built by `tools/oracle.sh`. The gate's `conformance` stage builds it and runs the runner; by hand:

```sh
tools/ci.sh conformance
# or
(cd rust && cargo run -p craftbridge-effects-conformance -- --oracle "$(../tools/oracle.sh)" --scenario T5)
```
