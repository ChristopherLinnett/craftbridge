# Review round B5 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B5 checked round A5's claims and fixes by running them, and reviewed the whole piece again: RESULT.md, the Swift probe, `ecprobe`, the scripts and the captures. It followed RESULT §8's capture method. The canonical captures are unchanged since round A4 (all 17 hashed before and after the round; `capture/a4-run/` equals `capture/` byte for byte), so B5 made no new copy and ran into `capture/b5-check/`.

EffectCraft was read only at the pinned rev `103760584a60074c9aaeef686d2d05d89f08155e`, through `git show` in the existing clone, plus the cargo registry copies of skrifa and read-fonts. Work was alone. No font was downloaded, no GitHub request was made, no git state was changed, nothing was posted, and no physical device was touched.

## 1. Environment and starting state

- The simulator is iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`: booted, iOS 26.5 (23F77), Xcode 26.6 (Build 17F113). `simctl list runtimes` shows that one runtime only (§6). Third-party apps before and after: `com.craftbridge.FontProbe` and `com.example.orchestratorFlutter`; FontProbe stayed installed (`run-sim.sh` reinstalls it), the other app was untouched, the simulator was left booted.
- **Device: the acceptance iPhone (iPhone 12 Pro) is still `unavailable`.** B5's `devicectl` capture reproduced `capture/review-devicectl.txt` byte-identically.
- All 22 SHA-256 values §8 lists matched before anything was touched (the 20 source files, the canonical report/stdout pair `f2844cb0…`, `review-host-checks.txt` `20cf25df…`). The kept run reports match their recorded hashes: `a1-run` `e05f2b64…`, `b1-run` `a4694a8f…`, `a2-run` `4c544c06…`, `a3-run` `0e8b07f0…`, and `a4-check` `952f17c2…`.
- The 17 canonical files (`capture/review-*`, `sandbox-canary.txt`) were hashed before the round and `shasum -c` verified after it: all 17 unchanged.

## 2. Round A5's claims and fixes, checked

| A5 claim or fix | How B5 checked | Result |
|---|---|---|
| fix 1: §3's timing sentence (17 ms canonical; 16 to 30 ms over the eight kept runs; scans 13 to 243 ms; timings called instance values) | pulled the timing lines from all eight kept reports | holds. `add_source` + `families()`: a1 25, b1 16, a2 16, a3 22, canonical 17, a4-check 16, b4-check 16, a5-check 30 ms — every value in 16 to 30. Scans over those runs: min 13 (b1, a2, b4-check), max 243 (a5-check Mac root). A5's table matches the reports line for line |
| fix 2: §4 reads "Not verified yet" | read §4; grep RESULT for "this round" | holds: the sentence is there and no stale "this round" wording remains anywhere in RESULT |
| §8's pattern line replaces the run/check-folder rows | read §8; grep RESULT for per-round folder names | holds: only `a1-run`, `b1-run`, `a2-run`, `a3-run` have rows (all cited above in §8 itself), one pattern line covers every other `<round>-run`/`<round>-check`; no concrete `b2-run`/`b3-run`/`a4-run`/`a4-check`/`b4-check`/`a5-check` names remain in RESULT; the two rows that pointed at `capture/a4-check/` now cite REVIEW-A4.md |
| "eight runs whose reports are kept" | enumerated every `review-documents-report.txt` under `capture/` | holds: 8 distinct runs; `b3-run`'s report is byte-identical to the canonical (a copy), `a4-run` is a copy, `b2-run` holds no report (only its re-fetched noto-cjk capture) |
| the canonical captures unchanged since A4, `a4-run/` = `capture/` | `cmp` of all 17 files | all byte-identical, so no copy was owed |
| a check run leaves `capture/` untouched | `shasum -a 256 -c` over the 17 canonical files after B5's run | all OK |
| B4's other carried claims (§5 step 3 only builds and prints; `FontProbe.xcodeproj` has no cargo build phase; a foreign `CARGO_TARGET_DIR` is never used) | B5's own check run + `grep -c PBXShellScriptBuildPhase` (= 0) + foreign `CARGO_TARGET_DIR` set for the whole run | all hold (below) |

## 3. The check run (`capture/b5-check/`)

B5 deleted `build/` and `ecprobe/target/`, then ran §5 steps 1 to 5 with `CAPTURE_DIR=capture/b5-check` and `CARGO_TARGET_DIR=/tmp/craftbridge-v1/b5-foreign-target` exported for the whole run. Step 6 was skipped: no GitHub requests were allowed this round, so `review-noto-cjk-sizes.txt` was not re-fetched and was checked from disk (§4). The foreign folder was never created: both scripts set their own `CARGO_TARGET_DIR` (`ecprobe/target`), as §5 says.

- `run-sim.sh`: exit 0; report 52,694 B, SHA-256 `2f88397a…`; `stdout == Documents copy`;
- `engine-checks.sh`: exit 0;
- iphoneos build: `** BUILD SUCCEEDED **`, arm64 Mach-O, exactly the 6 `T _ecprobe_` symbols, matching the 6 functions declared in `ecprobe/ecprobe.h` (step 3 wrote nothing into the capture folder, as §5's preamble says);
- typecheck: exit 1 with the one expected error (`CTFontManagerCopyAvailableFontURLs()` unavailable in iOS);
- `devicectl list devices`: exit 0, device `unavailable`, capture byte-identical.

Compared with `capture/` (15 files; `sandbox-canary.txt` stays in `capture/` and step 6 was skipped):
- **Byte-identical (11):** both folder lists, both family lists and their diff, the Arabic face names, the Han fallback, the outline formats, host-checks, the typecheck, devicectl.
- **Instance values only:** the report and stdout differ in 19 line pairs — timestamp, pid, the container-path lines including the XPC id, seven ms timings, the section-11 body byte count and SHA and the `READ-BACK` byte count; the `TERM = dumb` line is present in both this time, which is why the file delta is exactly −3 B (pid 40810→3384 and the two 3-digit scan timings each losing a digit; verified line by line). stderr and os_log carry timestamps, pid and that byte count.
- **Every count is the same:** 787/370/378, 613/264/296, 633/263/311, `faces before 7`, `families() now 314`, 303/103, `OK 103, FAIL 0`, 86/86/86 with `differ in: 0`, the whole per-folder table, the fallback answers.
- This run: scans 44 / 98 / 14 ms, `add_source` + `families()` 16 ms — inside the ranges §3 now records, and by §8's rule nothing was promoted: the canonical report and RESULT's run record (the B3 run, 52,697 B, `f2844cb0…`) stay as they are, and §3's sentence (scoped to the eight runs kept as of round A5) needed no change.

## 4. Other claims re-derived this round (no defect)

| RESULT claim | How B5 checked | Result |
|---|---|---|
| §2's section list and framing (line-by-line stdout, one os_log pointer line, Documents write + read-back, sections 1 to 11) | the report's own section headers; `FontProbe.swift` | match; the report prints each line to stdout as written, os_log gets one pointer line, section 11 writes and reads back |
| §3 section 1 to 6: 82 + 290 = 372; `st_dev` 16777232 / 16777239; `/Core` ENOENT 260; runtime layout 8 folders (37/74/5/27/1/106/2/14 = 266 = 264 sfnt + the two extensionless); the pair is `HelveLTMM` + `TimesLTMM`; no top-level files, nothing deeper, no symlinks; Helvetica 2,369,592 B vs 1,195,996 B; GeezaPro identical; magics `ttcf`/`00010000`/`OTTO`; the env lines (section 1) | the canonical report sections 1 to 6, `host-checks.txt`, and a fresh `find … ! -name '*.*'` on both roots | as written, on the Mac and the runtime copy |
| §3 section 8: 303 descriptors, 103 files in 8 folders (29/42/1/11/1/4/14/1), all under the runtime root, `OK 103, FAIL 0`, 55/47/1 magics, 352 MiB, 86/86/86 both `differ in: 0`, the six named-font files | re-read section 8 line by line | as written |
| §3 section 9: the four `dirs` cases; the per-folder table (sums to 103/263/633/311); `system()` returns the Linux list with none existing, from the `cfg!(target_os = "macos")` branch; `faces before 7` = the three families effectcraft-text bundles (Inter, JetBrains Mono, Noto Serif, per `fonts.rs` include list); 81/86 English + 5 Arabic-named (e.g. `گیزا پرو`); family arithmetic 314 = 3 + 92 + 219, 86 + 133 = 219, 133 + 3 + 5 = 141 (the 141 items counted from the report's own line); only PingFang SC and PingFang TC of the 20 Chinese-list names are among the 314; common parent `<RuntimeRoot>/System/Library` not walked | report section 9 + counting scripts over the report; `fonts.rs` at the pinned rev | as written, every figure reproduced from the captures |
| §3 section 10: 22/22, 16/16 (face 0 of 6), 20/20, 21/21 (face 20 of 32), 21/21, 13/13; CoreText reloads 5 of 6, 0 for rebuilt PingFang; rebuilds 58,965,676 B of 61,080,840 B and 139,052,436 B of 139,284,540 B; `add_font_data` dedups nothing while `add_source` skips family+style | report section 10; `fonts.rs` `add_source` (398-413) and `add_font_data` (415-422) | as written |
| §3's and §4's EffectCraft citations: `walk` 112-135 (breadth first, `depth < 8`, canonicalized once, ttf/otf/ttc/otc, file or missing folder skipped silently), `system()` 80-101, `FaceData::File` 40, name-tables-only scan 72, shared bytes per collection 209-212, bold synthesis 635-636, `METRIC_ALIASES` 658-659 (Calibri→Carlito), the Chinese lists 824-842, `asks_traditional_chinese` 971-973 (hk/mo included), `covers`/`glyph_draws` 1018-1052, the `has_char` skip 1117, `system_fallback` 1169-1174; skrifa 0.44.0 `OutlineGlyphFormat` = glyf/CFF/CFF2/VARC only and neither skrifa nor read-fonts mentions `hvgl` | `git show` at the pinned rev; the registry copies of skrifa 0.44.0 and read-fonts 0.41.0; skrifa 0.44.0 is also what EffectCraft's own `Cargo.lock` pins | every citation lands on the code RESULT describes |
| §4's Noto/CJK figures: the nine file sizes, 14.0 MB one weight / 28.4 MB two, VF pair 25.2 MB, `NotoSansSC[wght].ttf` 17,772,300 B with default instance Thin (post_script_name `NotoSansSC-Thin`), upstream note naming `Sans/Variable/TTF/Subset/NotoSansSC-VF.ttf`, `NotoSansCJKsc-Regular.otf` 16,437,364 B, OFL 1.1 licence lines | re-read `review-noto-cjk-sizes.txt` (no network this round); `b2-run`'s copy differs from canonical only in its fetch-time line, as §8 says | as written |
| §5's example contract: `@file`, plain folder, and exit 2 when a list cannot be read (`@missing` and `@` alone), a named folder does not exist, or nothing is named; absolute paths under `simctl spawn` | ran the freshly built `dirfamilies` against each case | all four exit-2 cases and the two ok cases reproduce; the `@missing` error even carries the absolute-path hint |
| §5/A4-B4 claim: a failed build stops `run-sim.sh` before anything installs, and the errors are printed | planted `let b5PlantedError: Int = "not an int"`; ran `run-sim.sh` into a scratch capture folder | exit 1 at the build step, `FontProbe.swift:33:35: error: cannot convert value of type 'String' to specified type 'Int'` printed, the capture folder stayed empty, nothing reinstalled; source restored and its SHA-256 `58a72c5c…` re-verified |
| §1/§6/§7 environment: one runtime; `devicectl` state; the `Catchable signals` quote; 257/260 in `FoundationErrors.h`; §3's CoreText doc quotes — "creates a new collection containing all fonts available to the current application" with `ios(3.2)`, "accessing the font URL from the font descriptor" `kCTFontURLAttribute` `ios(3.2)`, `ios(10.0)` for the two CopyAvailable names calls; `man 1 dyld` has no `DYLD_ROOT_PATH` and does carry "not found at its install path" | reran the commands and grepped the SDK headers and man page | as written |
| §8's per-line claims: five captures byte-identical to `b1-run`'s; canonical `host-checks` differs from `a3-run`'s by exactly the one new top-level/symlink line; the worker's `documents-report-final.txt` = `stdout-report.txt` (30,772 B); `oslog-run2.txt` cut at ~1 KB at `DYL<…>` (both report blocks measured: 996 B each) | `cmp`/`diff`/`wc`/tail measurement | all hold |

The §1 "Paired iPhone" row lists the rounds up to B3 that re-checked the device; A4, B4, A5 and B5 re-checked it too and each reproduced the capture byte-identically. The row is true as written (it does not claim exclusivity), so by §8's rule it is left alone.

## 5. Defects found in round B5, and fixes

**None.** Every figure, citation and behaviour claim that this round could run or read held — including both of A5's fixes and every A5 claim this round could re-run (the timing table, the kept-report enumeration, the pattern-line state of §8, and the untouched canonical captures). No defect was found to fix.

## 6. Agreement with the recommendation

**Agree, unchanged.** AD2 on iOS should take the parent folders of the files CoreText reports through `CTFontCollectionCreateFromAvailableFonts` + `kCTFontURLAttribute` and pass them to `add_source(&DirectorySource { dirs })`. Three alternatives are wrong:
- `/System/Library/Fonts` gives the Mac's fonts in the simulator and misses `CorePrivate`.
- File URLs are ignored by `walk`.
- The common parent of those folders is far too wide.

Those folders register 263 files where CoreText offers 103, which is a display question for FX-3. Chinese still needs a bundled CJK font, because PingFang's 32 faces are `hvgl`-only and skrifa 0.44.0 has no `hvgl` reader.

## 7. Open items

- **Device half of V1:** the acceptance iPhone is still `unavailable`; §7's recipe stands (with the 257/260 reading of section 7).
- **FX-3, what `list_fonts` shows:** the 92 dot-named and the 133 extra visible families. Not decided.
- **The bundled CJK font:** the owner's call. §4's noto-cjk proposal stands; the family names inside the files and the fallback drawing with them stay unverified until the files are downloaded.

## 8. Outside the workspace

- Read-only use outside: `git show` at the pinned rev in the EffectCraft clone, the cargo registry copies of skrifa and read-fonts, the SDK headers (`FoundationErrors.h`, `CTFontCollection.h`, `CTFontDescriptor.h`, `CTFontManager.h`), `man 1 dyld`, and the `simctl`/`devicectl` queries. No GitHub request, no download, no state-changing git command.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds. The simulator was left booted with FontProbe installed and the other app untouched.
- Scratch stayed inside `/tmp/craftbridge-v1`: `b5-scratch/` (the before/after canonical hashes, the run log, the planted-error source backup and log, the extracted `fonts.rs` and the hash-check script) was deleted after every restore and re-verify; the empty planted capture folder was removed; `b5-foreign-target` was never created. `capture/b5-check/` is this round's check evidence and needs no §8 row under the pattern line.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` (the brief, read only) and `/tmp/craftbridge-v7` were not touched.

**Verdict: clean: no findings.**
