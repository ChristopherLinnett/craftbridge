# EffectCraft conformance corpus

The scenarios (T1–T24 of the plan, §28) that every platform will run against the EffectCraft command line, which is the oracle, together with their fixtures (`fixtures/`) and, later, the running example's template recipe. The data here belongs to no platform: the desktop runner (`rust/effects/craftbridge-effects-conformance`), the Swift tests (`ios/sdk/Tests`, which run the plug-in's side of each scenario on the iPhone simulator) and the Android instrumented tests (`android/sdk/effects/src/androidTest`, which run it on the Android emulator, the build packing this folder into the test APK as it is) all read it, so a scenario is written once. Each of the three refuses a key or a kind it does not read, so a key, a kind or a comparison the corpus gains is read by all three (`corpus.rs`, `Corpus.swift`, `Corpus.kt`) in the same change. T1 to T22 and T24 are here, T22 on the plug-in alone, since the command line stops a render only by being killed.

`corpus.toml` holds the scenarios, and its header says how one is written: steps (one oracle invocation each, and the calls that make the same step on the plug-in) and what is checked after them, on the oracle and on the plug-in, whose output and files a comparison holds against the oracle's. The desktop runner starts each side as a process in one cleared environment that holds only its own configuration folder, so nothing in the shell running it, such as fonts in `HOME`, thread counts or GPU switches, changes what either renders. It compares what the two produce with each other and with what the corpus says they must, and for an encoded export applies D-12: the same frame count and size, and every decoded frame within 40 dB PSNR. A frame the GPU renders is held to the compositor's own tolerance against the CPU's. A PNG sequence is held to the same files byte for byte, a GIF to the same decoded frames pixel for pixel, and a cancelled export to no file left behind.

The oracle is the official EffectCraft command line at the pinned revision, built by `tools/oracle.sh`. The gate's `conformance` stage builds it and runs the runner; by hand:

```sh
tools/ci.sh conformance
# or
(cd rust && cargo run -p craftbridge-effects-conformance -- --oracle "$(../tools/oracle.sh)" --scenario T5)
```
