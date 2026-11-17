# REVIEW-A1 — review round 1 of craftbridge#28 (V1)

I reviewed the whole build round: the probe, the Xcode project, the captures and RESULT.md. I re-ran what it claims, fixed what was wrong or missing, and rewrote RESULT.md, keeping the worker's correct findings.

## Defects found

| # | Severity | Where | Defect | Fix |
|---|---|---|---|---|
| 1 | major | RESULT verdict, §6, §7 | Calls the app "sandboxed" and says the simulator "proves the sandbox policy in the simulator allows it". It proves nothing of the kind. The simulator app read a plain Mac file, `/tmp/craftbridge-v1/capture/sandbox-canary.txt`, outside every container (new section 7: `READ ... OK`). No iOS sandbox limits its file access. | New section 7 in the probe; verdict and §3 rewritten. |
| 2 | major | RESULT §3 and §4.2 | Says the report was "echoed verbatim into os_log" and that `log show` "shows the full report between begin/end". False: os_log cut each report off after about 1 KB, at `DYL<…>`. `capture/oslog-run2.txt` has 2 cut-off markers and 0 `report end` lines. | The probe now logs one pointer line to os_log (`capture/review-oslog.txt`). The report goes to stdout line by line and to Documents. RESULT corrected. |
| 3 | major | RESULT §6 design note | Says a scan of `/System/Library/Fonts` "must recurse at least one level", as if CraftBridge needed its own scanner. EffectCraft's `walk` (`crates/text/src/fonts.rs:112-135`) already goes breadth first, keeps descending while `depth < 8`, and visits each canonical folder once. Measured: `DirectorySource { dirs: [<RuntimeRoot>/System/Library/Fonts] }` finds 264 files and 613 faces, though that folder has no font files at its top level. What the note leaves out: a **file** in `dirs` is skipped without any error (0 faces). | RESULT §3 states the real walk behaviour, with the measurements. |
| 4 | major | missing | Never asked CoreText, so it never saw that the iOS fonts are reachable in the simulator. `CTFontCollectionCreateFromAvailableFonts` + `kCTFontURLAttribute` give 303 faces in 103 files, all under the runtime root. All are readable. They sit in 8 folders, one outside `Fonts/` (`PrivateFrameworks/FontServices.framework/CorePrivate`, PingFang). | Probe section 8. |
| 5 | major | missing | Never checked what EffectCraft itself does on iOS. `DirectorySource::system()` picks the Linux folders on iOS (`cfg!(target_os = "macos")` is false there), and none of them exist, so EffectCraft finds no system fonts without AD2. In the simulator, a hard-coded `/System/Library/Fonts` registers the **Mac's** 378 families. FX-3's acceptance check ("`list_fonts` shows the system families on the simulator") would then pass with the wrong fonts. | `ecprobe` (the real `effectcraft-text` at 10376058) linked into the app; probe section 9. |
| 6 | major | missing | iOS 26.5's PingFang (`PingFangUI.ttc`, 32 faces) has only Apple `hvgl` outlines. It is the only such file among the runtime's 645 faces. EffectCraft cannot draw it: `has_char('汉')` is false although the cmap maps 汉. In the simulator, EffectCraft's fallback gives nothing for Simplified 汉 and Hiragino Sans (Japanese forms) for Traditional 漢, whichever folders AD2 passes. | `capture/review-outline-formats.txt` and `capture/review-han-fallback.txt`; RESULT verdict and §4. |
| 7 | minor | RESULT verdict, §6 | Says 372 "font files" and 266 runtime "font files". Each count includes `HelveLTMM` and `TimesLTMM`, two extensionless files that EffectCraft skips. The real counts are 370 and 264, confirmed by the real `DirectorySource`. | Corrected. |
| 8 | minor | RESULT §6 | Unverified claim: "Historically iOS has exposed /System/Library/Fonts as publicly readable (which is why bundling fonts has always been optional)". The reasoning does not follow, and no source is given. | Removed. The device question stays open, with a run recipe (RESULT §7). |
| 9 | minor | probe | Copied code: section 5 re-listed every subfolder with a second copy of section 3's loop. | Shared helpers (`list`, `readHead`, `describeMagic`), one listing (`FontTree`). |
| 10 | minor | probe | The verdict append used `seekToEndOfFile()` and `write(_:)`. Those raise Objective-C exceptions instead of throwing, so the probe's `catch` ("APPEND-VERDICT: FAIL") could never run. | Replaced with the throwing `seekToEnd()` and `write(contentsOf:)`. |
| 11 | minor | probe | The whole report went to stdout at the end, so a crash or hang in any section would lose everything. That happened in this round: a too-broad walk hung the run. | Each line goes to stdout as it is written. stdout and the Documents file are still byte-identical (`cmp`). |
| 12 | minor | project | Not ready for the acceptance iPhone: no run recipe, warnings about missing launch screen and orientations (iPad), and no device library. | Added `UILaunchScreen`, made the app iPhone only, linked `ecprobe` per SDK, and confirmed the `iphoneos` build links (unsigned) with no warnings. Recipe in RESULT §7. |

The worker's other findings hold, and I reproduced them independently (`capture/review-host-checks.txt`):
- The app's 372 paths are identical to the Mac's `/System/Library/Fonts` two levels deep, and different from the runtime copy.
- `st_dev` is 16777232 for the app's folder and 16777239 for the runtime copy.
- `Core/` gives ENOENT inside the app.
- The runtime copy has 8 subfolders and no top-level files.
- The magic bytes are right.
- iOS 26.5 is the only installed runtime.
- The worker's source SHA-256s match the copies kept in `capture/worker-sources/`.

## What I ran

- Read `crates/text/src/fonts.rs` and `font_dirs.rs` at 10376058 with `git show`, and searched for `scan_system` callers with `git grep` (read-only).
- Ran `xcrun swiftc -typecheck` on `review-scripts/coretext-availability.swift` against the iOS 26.5 simulator SDK. Only `CTFontManagerCopyAvailableFontURLs` fails: `CT_UNAVAILABLE(ios, watchos, tvos)`. Output in `capture/review-typecheck-ios.txt`.
- Built `ecprobe` with cargo for `aarch64-apple-ios-sim` and `aarch64-apple-ios`. It depends on `effectcraft-text` through a `git = "file://…/effectcraft/repository"`, `rev = 1037605…` dependency, which cargo fetches read-only into `~/.cargo/git/db/repository-9f1ad1fe48d3a199`. Its lock file starts from EffectCraft's `Cargo.lock`.
- `review-scripts/run-sim.sh`: builds both, builds FontProbe in Release, installs, launches with `--stdout`/`--stderr`, collects the Documents copy, `cmp`s it with stdout, and runs `host-checks.sh`. Final run: pid 35566, 46,188 B, SHA-256 `e05f2b64…48adc`.
- Built FontProbe for `iphoneos` with `CODE_SIGNING_ALLOWED=NO`: BUILD SUCCEEDED, arm64, `ecprobe_` symbols present.
- On the Mac, with the same EffectCraft code: `ecprobe/examples/dirfamilies.rs` on the runtime's `Fonts` root and on CoreText's folders (`capture/review-families-*.txt`: 296 vs 311 families; the difference is PingFang and the Watch fonts), and `examples/faces.rs` on the Arabic fonts, both the iOS and the Mac copies.
- In the simulator: `xcrun simctl spawn booted …/examples/fallback` with both folder lists (`capture/review-han-fallback.txt`).
- `python3 -I review-scripts/outline-formats.py <RuntimeRoot>`, a scan of the table directory of every runtime face (`capture/review-outline-formats.txt`).
- `simctl spawn booted log show` for the os_log lines, and the os_log check of the worker's `oslog-run2.txt`.

## Answers

1. **Host vs runtime: true.** In the simulator, `/System/Library/Fonts` is the Mac's folder. The evidence is the identical 372-path list, `st_dev` 16777232 against 16777239 for the runtime copy, `/System/Library/Fonts/Core` giving ENOENT, and different bytes for fonts with the same name (Helvetica: 2,369,592 B on the Mac, 1,195,996 B on iOS). In addition, the simulator applies no iOS sandbox at all (canary), so it says nothing about a device's sandbox.
2. **CoreText reports the iOS runtime's fonts.** It gives 303 descriptors in 103 files: 103 under `<RuntimeRoot>`, 0 under the Mac's folder. The app reads all 103 (0 failures, 352 MiB). `UIFont.familyNames` = `CTFontManagerCopyAvailableFontFamilyNames` = the collection's families = 86. The system UI font is `<RuntimeRoot>/System/Library/Fonts/Core/SFUI.ttf`. `CTFontCopyAvailableTables` and `CTFontCopyTable` return every table, byte-identical to the font's file, for six samples (TrueType, collection, CFF, colour emoji, system UI font, PingFang). The rebuilt sfnt is accepted by EffectCraft's `add_font_data`, so font data is available without file access. It costs a full copy of each font in memory, though (Emoji 139 MB, PingFang 59 MB).

## Recommendation for AD2 on iOS

- Build `dirs` from CoreText. Take `CTFontCollectionCreateFromAvailableFonts(nil)`, get each descriptor's `kCTFontURLAttribute`, take the distinct **parent folders**, and pass them to `add_source(&DirectorySource { dirs })`. Do not hard-code `/System/Library/Fonts`.
- Evidence, in the simulator: 633 faces and 311 families, matching `UIFont.familyNames` (81 of 86 by English name, the other 5 under their Arabic names, as EffectCraft also names the Mac copies), in 25 ms. The hard-coded folder gives the Mac's fonts in the simulator, and on the iOS layout it misses the `FontServices` folder and adds Watch-only fonts.
- Do not pass the file URLs (ignored) or their common parent (`/System/Library`, too wide for the 8-level walk).
- Fallback, only if the device run shows that CoreText's files cannot be read: `CTFontCopyTable` → sfnt → `add_font_data`, per family on demand, plus R3's bundled fonts.
- Separately: Chinese needs a bundled CJK font in `Folders.fonts`, because EffectCraft cannot draw iOS 26's `hvgl` PingFang. That is a decision for the plan, as an actionable issue.

## Not done / left open

- **Device half of V1:** the paired iPhone 12 Pro, the acceptance iPhone, was `unavailable`. Run RESULT §7 on the acceptance iPhone. It needs a signing team.
- `gh issue view 28 -R ChristopherLinnett/craftbridge` could not find the repository with the current `gh` login, so I took V1's wording from the plan's `EFFECTCRAFT-PLUGIN-PORT-SCOPING.md` (read-only `grep`; nothing in `artcraft-plugin-port` was changed). Nothing was posted.
- Files outside `/tmp/craftbridge-v1`: cargo's cache of the EffectCraft clone (`~/.cargo/git/db/repository-9f1ad1fe48d3a199`, plus its checkout under `~/.cargo/git/checkouts`), and `fontprobe-stdout.txt` / `fontprobe-stderr.txt` in the simulated device's `data/tmp`, where `run-sim.sh` points `simctl launch --stdout` (simctl resolves those paths inside the device). I removed my earlier stray files there.
- 92 of the 314 families EffectCraft lists on iOS are Apple's hidden dot-named fonts. EffectCraft 10376058 does not filter them. This is a display question for FX-3.
