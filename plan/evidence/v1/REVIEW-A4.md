# Review round A4 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round A4 checked every claim and fix of round B3 by running it, and reviewed the whole investigation again: RESULT.md, the Swift probe, `ecprobe`, the scripts and the captures. It fixed what was wrong in place and used the new capture method. That method is now written once in RESULT §8:
- copy the canonical captures aside first;
- run into a scratch folder and compare;
- promote a capture only when its content really changes.

EffectCraft was read only at the pinned rev through `git show` in the existing clone. Work was alone. No font was downloaded, no live GitHub request was made, no git state was changed, and nothing was posted.

## 1. Environment and starting state

- The simulator is iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`: booted, iOS 26.5 (23F77), with Xcode 26.6.
- Third-party apps before and after the round: `com.craftbridge.FontProbe` and `com.example.orchestratorFlutter`. FontProbe stayed installed (`run-sim.sh` reinstalls it), the other app was not touched, and the simulator was left booted.
- **Device: "SeaEl iPhone" (iPhone 12 Pro) is `unavailable`** in `xcrun devicectl list devices`. The check capture is byte-identical to the canonical one. Nothing was installed or run on a device.
- Before running anything, round A4 copied B3's `capture/review-*` files and `sandbox-canary.txt` (17 files) to `capture/b3-run/`.

## 2. Round B3's claims and fixes, checked

| B3 claim or fix | How A4 checked | Result |
|---|---|---|
| §8 hashes | `shasum -a 256` on every file §8 lists | all 22 matched before A4 changed anything |
| `capture/a3-run/` holds A3's captures | report hash, and a comparison with the canonical captures | A3's report `0e8b07f0…` is intact; A3's other captures match the canonical ones wherever no instance value is involved |
| fix 1: the `noto-cjk-sizes.py` docstring counts four excerpted files | read it | it names the licence, both `METADATA.pb` files and the upstream note, matching the code. No live run this round, to keep GitHub requests at zero; A3 and B3 already ran it today |
| fix 2: RESULT records B3's run as canonical (52,697 B, `f2844cb0…`) | hashed the report; `diff` against A3's | true. Against A3, only instance values, ms timings and B3's `TERM = dumb` line differ. Every count is identical |
| `run-sim.sh` stops on a compile error and prints it | planted `let a4PlantedError: Int = "not an int"` (source backed up, restored, SHA-256 `58a72c5c…` verified) after A4's script change | exit 1 at the build step; the `FontProbe.swift:295:27: error:` line was printed; nothing was installed; the check folder stayed empty |
| a foreign `CARGO_TARGET_DIR` is never used | check run with `CARGO_TARGET_DIR=/tmp/craftbridge-v1/a4-foreign-target` | that folder was never created |
| REVIEW-B3 §4: "`FontProbeEngine.registerFontFolders` is exactly that shape" | grepped the sources | **no such function exists**; the probe's registration is `effectCraftRegistration` calling `ecprobe_register_dirs`. This is in B3's notes, not in RESULT, so nothing in RESULT needed fixing |

## 3. The check run (`capture/a4-check/`)

I deleted `build/` and `ecprobe/target/`, then ran, with `CAPTURE_DIR=capture/a4-check`, the planted-error test and §5 steps 1 to 5:
- `run-sim.sh` exit 0; stdout == Documents copy; 52,681 B;
- `engine-checks.sh` exit 0;
- iphoneos build `BUILD SUCCEEDED`, 6 `T _ecprobe_` symbols;
- typecheck exit 1 with the one expected error;
- devicectl `unavailable`.

Step 6 was skipped to make no GitHub request.

Compared with the canonical captures:
- **Byte-identical:** the Arabic face names, both folder lists, both family lists and their diff, the outline formats, the typecheck, devicectl.
- **Instance values only** (no change by the rule):
  - the report and stdout: pids, timestamps, container paths, the XPC id, ms timings, and no `TERM` line (16 bytes, so 52,681 B against 52,697 B);
  - stderr and os_log: timestamps, pid and that byte count.
  
  Every count is the same: 787/370/378, 613/264/296, 633/263/311, 314 families, 303/103/8, 645/32, 86 = 86 = 86, the per-folder table, the fallback answers.
- **Content changed by this round's own script changes**, and promoted to `capture/`:
  - `review-host-checks.txt`: one new line, `files at its top level: 0; symlinks anywhere in it: 0`;
  - `review-han-fallback.txt`: `registered … from @review-fontsroot-dir.txt` names the list by file name. The answers are unchanged.

So the canonical report and RESULT's run record (§3's "17 ms in this run", §8's B3 report line) stay as round B3 left them.

## 4. Changes the new method needed (not counted as defects)

- **The scripts could only write into `capture/`,** so a check run would have overwritten the canonical captures. `run-sim.sh` and `engine-checks.sh` now write into `$CAPTURE_DIR` (default `capture/`), and `engine-checks.sh` reads the report from there too. The sandbox canary the app reads stays in `capture/`. §5 steps 4 to 6 use `${CAPTURE_DIR:-capture}`.
- **`fallback`'s `registered … from` line printed the folder list's absolute path,** so its capture would differ in any other folder. A new `ecprobe::arg_label` names a list by its file name; it sits beside `folders_from_args_or_exit` with one `LIST_PREFIX`. `cargo clippy --all-targets` is clean for the host and the simulator target.
- **RESULT §8** states the method once.
- **RESULT §5's rounds paragraph** says that later check runs are recorded in each round's REVIEW file, which ends the per-round re-pointing.

## 5. Defects found in round A4, and fixes

1. **minor — RESULT §3's "no top-level files … and no symlinks" had no capture behind it.** The top-level count could only be worked out from host-checks' totals, and the symlink count appeared only in B1's and B3's review notes. **Fix:** `host-checks.sh` now records both, as `files at its top level: 0; symlinks anywhere in it: 0` in `capture/review-host-checks.txt`.
2. **minor — RESULT §7 predicted an untested device result.** It said section 7 "should say the canary file does not exist", but on a device the sandbox may refuse `/tmp/craftbridge-v1/…` before the lookup. **Fix:** it now says the read must fail, and that error 257 (`NSFileReadNoPermissionError`) or 260 (`NSFileReadNoSuchFileError`) tells which. Both codes are quoted from the SDK's `FoundationErrors.h`. The sandbox answer itself stays with section 8.
3. **minor — RESULT §5 never said where `effectcraft-text` comes from.** `ecprobe/Cargo.toml` uses an absolute `file:///Users/christopherlinnett/dev/artcraft-dev/effectcraft/repository` URL, so on another machine the first cargo build fails. **Fix:** §5 says so and what to change; the `rev` and `Cargo.lock` still pin the code.

No other defect was found. Every RESULT figure, citation and behaviour claim that the check run or a reading could test held.

## 6. Agreement with the recommendation

**Agree, unchanged.** AD2 on iOS should take the parent folders of the files CoreText reports and pass them to `add_source(&DirectorySource { dirs })`:
- not `/System/Library/Fonts`, which gives the Mac's fonts in the simulator and misses `CorePrivate`;
- not the file URLs, which `walk` ignores;
- not their common parent, which is far too wide.

Those folders register 263 files where CoreText offers 103, which is a display question for FX-3. Chinese still needs a bundled CJK font, because PingFang's `hvgl` outlines are unreadable to skrifa 0.44.0.

## 7. Open items

- **Device half of V1:** "SeaEl iPhone" is still `unavailable`; §7's recipe stands.
- **FX-3, what `list_fonts` shows:** the 92 dot-named families and the 133 extra visible families. Not decided.
- **The bundled CJK font:** the owner's call. §4's noto-cjk proposal stands. The family names inside the files and the fallback drawing with them stay unverified until the files are downloaded.

## 8. Outside the workspace

- Read-only use outside: `git show` in the EffectCraft clone, and the SDK headers.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds.
- Scratch stayed inside `/tmp/craftbridge-v1`: `a4-scratch/` held the source backup and the run log and was deleted after the restore was verified. `a4-foreign-target` was never created.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 3 defects fixed (0 major, 3 minor); recommendation agreed; device unavailable, not run.**
