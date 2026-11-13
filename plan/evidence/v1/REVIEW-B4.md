# Review round B4 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B4 checked every claim and fix of round A4 by running it, and reviewed the whole piece again: RESULT.md, the Swift probe, `ecprobe`, the scripts and the captures. It followed the capture method of RESULT §8: copy the canonical captures aside first, run into a scratch folder, compare, and promote a capture or change RESULT only when a count, a finding or a capture's content really changes.

EffectCraft was read only at the pinned rev `103760584a60074c9aaeef686d2d05d89f08155e`, through `git show` in the existing clone, plus the cargo registry copies of skrifa and read-fonts. Work was alone. No font was downloaded, no GitHub request was made, no git state was changed, nothing was posted, and no physical device was touched.

## 1. Environment and starting state

- The simulator is iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`: booted, iOS 26.5 (23F77), with Xcode 26.6 (Build 17F113). `simctl list runtimes` still shows that one runtime only (RESULT §6).
- Third-party apps before and after the round: `com.craftbridge.FontProbe` and `com.example.orchestratorFlutter`. FontProbe stayed installed (`run-sim.sh` reinstalls it), the other app was not touched, and the simulator was left booted.
- **Device: "SeaEl iPhone" (iPhone 12 Pro, `08452DF6-1739-55D5-8C38-622DEDB24E5E`) is still `unavailable`.** Nothing was installed or run on a device; `capture/review-devicectl.txt` was reproduced byte-identically.
- Before running anything, B4 copied A4's `capture/review-*` files and `sandbox-canary.txt` (17 files) to `capture/a4-run/` and hashed all 31 canonical capture files. Those two hashes (before and after the round) are what §8's method turns on.
- **§8's hashes:** every SHA-256 §8 lists (22 of them) matched before anything was touched, and the five kept run reports match what §8 says of them: `a1-run` 46,188 B `e05f2b64…`, `b1-run` 46,203 B `a4694a8f…`, `a2-run` 52,582 B `4c544c06…`, `a3-run` 52,681 B `0e8b07f0…`, `a4-check` 52,681 B `952f17c2…`.

## 2. Round A4's claims and fixes, checked

| A4 claim or fix | How B4 checked | Result |
|---|---|---|
| §8 hashes and the run folders | `shasum -a 256` on every file §8 lists, and on the five kept reports | all matched (above) |
| fix 1: `run-sim.sh` and `engine-checks.sh` write into `$CAPTURE_DIR` | check run with `CAPTURE_DIR=capture/b4-check`; then a proof of the default with the 17 canonical files backed up first and both scripts run with `CAPTURE_DIR` unset | holds. The check run wrote everything into `capture/b4-check/` and left `capture/` alone; with the variable unset both scripts wrote into `capture/` (engine-checks byte-identically, `run-sim.sh` a fresh 52,696 B report, re-measured on a second default run so the figure is not remembered), and the backup was restored and every canonical hash re-verified |
| a check run leaves every canonical capture byte-identical | `shasum -a 256 -c` over all 31 canonical files, before and after the run | all pass; nothing in `capture/` changed |
| fix 2: `ecprobe::arg_label` names a folder list by file name, so `capture/review-han-fallback.txt` was promoted | re-ran `engine-checks.sh`; also ran the host `fallback` example with `@./b4-scratch/one-dir.txt`, `@/nonexistent/x.txt`, `@` alone and a plain folder | holds. The capture is byte-identical to the canonical one, the `registered … from @review-fontsroot-dir.txt` lines name the list by file name, a plain folder prints as given, and an unreadable list exits 2. Answers unchanged since B1. `cargo clippy --all-targets` is clean for the host, `aarch64-apple-ios-sim` **and** `aarch64-apple-ios` (A4 checked the first two; B4 checked the device target too) |
| fix 3: `host-checks.sh` records the top-level file and symlink counts (both 0) | diffed the canonical capture against the copies in `a1-run`, `b1-run`, `a2-run`, `a3-run` | holds. Those four are byte-identical (1,918 B) and the canonical one (1,974 B) is one line longer: `files at its top level: 0; symlinks anywhere in it: 0`. REVIEW-B2.md records B2's copy as identical to A2's, so §8's "every other line byte-identical across the A1 to A4 runs" is right |
| fix 4: §7 now says the device read must fail, with 257 or 260 telling which | grepped `FoundationErrors.h` in `iphonesimulator26.5.sdk` | holds: `NSFileReadNoPermissionError = 257`, `NSFileReadNoSuchFileError = 260`, as §7 quotes |
| fix 5: §5 says what to change on another machine | read §5, `ecprobe/Cargo.toml` and `Cargo.lock` | holds: §5 names the absolute `file:///Users/christopherlinnett/dev/artcraft-dev/effectcraft/repository` URL and says to point it at any clone holding the pinned rev; the `rev` and the lock's source line fix the code |
| A4 §2: a failed build stops `run-sim.sh`, and a foreign `CARGO_TARGET_DIR` is never used | planted `let b4PlantedError: Int = "not an int"` (source backed up, restored, SHA-256 `58a72c5c…` re-verified), then ran §5 with `CARGO_TARGET_DIR=/tmp/craftbridge-v1/b4-foreign-target` | holds. Exit 1 at the build step, `FontProbe.swift:34:35: error: cannot convert value of type 'String' to specified type 'Int'` printed, nothing installed, the check folder stayed empty. The foreign folder was never created — `FontProbe.xcodeproj` has no build phase that calls cargo, so `xcodebuild` cannot reach it either |

## 3. The check run (`capture/b4-check/`)

I deleted `build/` and `ecprobe/target/`, then ran §5 steps 1 to 5 with `CAPTURE_DIR=capture/b4-check`, a foreign `CARGO_TARGET_DIR` and `TERM=dumb`:

- `run-sim.sh` exit 0 in 35 s; report 52,697 B; `stdout == Documents copy`;
- `engine-checks.sh` exit 0;
- iphoneos build `** BUILD SUCCEEDED **`, arm64 Mach-O, 6 `T _ecprobe_` symbols (`nm`, confirmed by `lipo -archs` and `file`);
- typecheck exit 1 with the one expected error (`CTFontManagerCopyAvailableFontURLs()` unavailable on iOS);
- `devicectl list devices` exit 0, device `unavailable`.

Step 6 was skipped to make no GitHub request; its capture and §4's figures were checked from the copy on disk instead (below).

Compared with the canonical captures:

- **Byte-identical (11):** both folder lists, both family lists and their diff, the Arabic face names, the Han fallback, the outline formats, host-checks, the typecheck, devicectl.
- **Instance values only** (no change by the rule): the report and stdout differ in 19 line pairs — the timestamp, the pid, six container-path lines including the XPC id, seven ms timings, the body SHA (the body carries the timestamp and paths) and the two Documents path lines; stderr and os_log carry their timestamps and pid. The report body is 52,043 B in both runs. **Every count is the same:** 787/370/378, 613/264/296, 633/263/311, 314 families, 303/103/8, 645/32, 86 = 86 = 86, the per-folder table, the fallback answers.
- **Timings** sit inside what RESULT records: `add_source` 16 ms (RESULT: 16 to 25 ms in every run whose report is kept), the three scans 103 / 103 / 13 ms (RESULT: 13 to 126 ms).

Nothing needed promoting, so the canonical report and RESULT's run record (the B3 run, 52,697 B, `f2844cb0…`) and §3's numbers stay as they are. That is what §8 prescribes, not a defect.

## 4. Other claims re-derived this round (no defect)

| RESULT claim | How B4 checked it | Result |
|---|---|---|
| §3's per-folder table (8 rows, `all 8` row) | line by line against section 9 of the report | identical; the columns sum to 103 / 263 / 633 / 311 |
| §3: the root "misses 22 families … adds 7 Watch-only families" | counted `capture/review-families-diff.txt` | 4 visible PingFang + 18 hidden `.PingFang …` = 22; 6 hidden `… Time` + Sukhumvit Set = 7 |
| §3: of the 133 extra visible families, "102 from UnicodeSupport … and 31 from AppFonts, Core, CoreUI and CoreAddition" | attributed section 9's 141-family list to the files behind each family | 102 + 31 = 133 exactly (the remaining 5 of the 141 are the Arabic-named families, 3 more are the bundled fonts) |
| §3: the two extensionless files are `HelveLTMM` and `TimesLTMM`; 372 − 2 = 370 | `find /System/Library/Fonts -maxdepth 2 -type f ! -name '*.*'` | exactly those two, on the Mac as on the runtime copy |
| §3's section-5 magics (`ttcf`, `00010000`, `OTTO`) and section 7's canary line | read the report | as written; the canary is 87 B and reads back in full |
| §3: 645 runtime faces, 32 with `hvgl`, 32 with nothing EffectCraft draws, all in `PingFangUI.ttc` | `capture/review-outline-formats.txt` | as written |
| §4's nine Noto/CJK file sizes | matched each against `capture/review-noto-cjk-sizes.txt` | all nine present (no network used) |
| §4's CoreText picks (`.PingFangUITextSC-Regular`, `.PingFangUITextTC-Regular`, `.HiraKakuInterface-W4`, `.AppleSDGothicNeoI-Regular`) | section 9 of the report | as written |
| §1 and §5's build facts (deployment target 17.0, iPhone only, no shell-script phase, 6 exported symbols, `ecprobe`'s absolute git URL) | `project.pbxproj`, `nm`, `Cargo.toml`, `Cargo.lock` | as written |
| §2, §6, §7's capture-backed claims (82 + 290 = the same 372 files `find` lists; one runtime; the exact `devicectl` line and the "Catchable signals" quote; `oslog-run2.txt` cut at `DYL<…>`; the worker captures' sizes and the `documents-report-final.txt` = `stdout-report.txt` equality) | re-measured | as written |
| §2 and §3's EffectCraft citations (`walk` 112-135, `system()` 80-101, `FaceData::File` 40/72/209-212, `add_font_data` 415-422, bold synthesis 635-636, `METRIC_ALIASES` 658-659, the Chinese lists 824-842, the `asks_traditional_chinese` test 971-973, `covers`/`glyph_draws` 1018-1052, the skip at 1117, `system_fallback` 1169-1174), and skrifa 0.44.0's `OutlineGlyphFormat` with no `hvgl` | `git show` at the pinned rev and the registry copies | every citation lands on the code RESULT describes |

## 5. Defects found in round B4, and fixes

1. **minor — RESULT §8's file table did not account for the folders §8's own method creates.** Every earlier copy folder is listed (`b1-run` … `b3-run`, `a4-check`), but the copy this method makes first — A4's captures, copied to `capture/a4-run/` before anything ran — had no line, and no later check folder would. **Fix:** two rows, `capture/a4-run/` and `capture/b4-check/`, the second pointing at this file. This is inventory only: no capture was promoted, no count changed, and B4's run record stays here as §8 requires.
2. **minor — RESULT §5's preamble overstated its own recipe.** It said "Every step writes into `${CAPTURE_DIR:-capture}`", but step 3 (the unsigned device build and the `nm` of it) writes nothing into the capture folder: it builds into `build/` and prints, and §5 shows its expected result as terminal output. **Fix:** the sentence now scopes itself to the steps that capture and says step 3 only builds and prints.

No other defect was found. Every RESULT figure, citation and behaviour claim that this round could run or read held, including all five of A4's items.

## 6. Agreement with the recommendation

**Agree, unchanged.** AD2 on iOS should take the parent folders of the files CoreText reports (`CTFontCollectionCreateFromAvailableFonts` + `kCTFontURLAttribute`) and pass them to `add_source(&DirectorySource { dirs })`:

- not `/System/Library/Fonts`, which in the simulator gives the Mac's fonts and misses `CorePrivate`;
- not the file URLs, which `walk` ignores;
- not their common parent, which is far too wide.

Those folders register 263 files where CoreText offers 103, which is a display question for FX-3. Chinese still needs a bundled CJK font: PingFang's 32 faces are `hvgl`-only and skrifa 0.44.0 cannot read them, so EffectCraft's fallback answers Inter for 汉.

## 7. Open items

- **Device half of V1:** "SeaEl iPhone" is still `unavailable`; §7's recipe (including the 257/260 reading of section 7) stands and needs a device.
- **FX-3, what `list_fonts` shows:** the 92 dot-named families and the 133 extra visible ones. Not decided.
- **The bundled CJK font:** the owner's call. §4's noto-cjk proposal stands; the family names inside the files and the fallback drawing with them stay unverified until someone downloads them.

## 8. Outside the workspace

- Read-only use outside: `git show` at the pinned rev in the EffectCraft clone, the cargo registry copies of skrifa and read-fonts, and the SDK headers (`FoundationErrors.h`, the CoreText header). No GitHub request, no download.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds. The simulator was left booted with FontProbe installed and the other app untouched.
- Scratch stayed inside `/tmp/craftbridge-v1`: `b4-scratch/` (source backup, canonical hashes and backup, run logs, the extracted `fonts.rs`) and `b4-hashcheck.sh` were deleted after the restores were verified; `capture/b4-planted/` was removed empty after the planted-error test; `b4-foreign-target` was never created.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 2 defects fixed (0 major, 2 minor); recommendation agreed; device unavailable, not run.**
