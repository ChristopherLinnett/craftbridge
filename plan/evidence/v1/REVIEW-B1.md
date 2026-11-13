# Review round B1 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B1 reviewed the whole piece of work (the worker's build round plus round A1's corrections), re-ran everything round A1 claimed, fixed what was wrong or missing, and reports here. Method: run it, don't argue with it. Work was alone; nothing outside `/tmp/craftbridge-v1` was modified; the EffectCraft repository was read only through `git show 10376058:crates/text/src/fonts.rs` (never checked out).

**Verdict: the findings stand. I re-verified every claim in RESULT.md by running it, agree with the recommendation, and fixed the defects below.**

## 1. Environment re-check

- Simulator iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`, iOS 26.5 (23F77), only installed runtime — unchanged.
- Xcode 26.6 (17F113) — unchanged.
- **Paired iPhone "SeaEl iPhone" (iPhone 12 Pro) was still `unavailable` per `xcrun devicectl list devices`, so the §7 device recipe was NOT run — the device half of V1 stays open.** The §7 commands were sanity-checked against `devicectl device install/process launch/copy from --help` (all flags exist as written).

## 2. Everything re-run, with results

| Round A1 claim | How B1 checked | Result |
|---|---|---|
| `run-sim.sh` reproduces the final run from a clean build | Deleted `build/` and `ecprobe/target/`, ran `run-sim.sh` end to end (twice: once before, once after the §3 fix below). Fresh cargo build of `effectcraft-text` at pinned rev 10376058, fresh xcodebuild, fresh install | exit 0; stdout == Documents copy; **all counts reproduce exactly**: Mac 372 files / 370 loadable / 787 faces / 378 families; runtime Fonts root 613/264/296; CoreText dirs 633/263/311; `add_source` added 633, `families()` 314 (92 dot-named); UIFont match 81 English + 5 Arabic-named via `add_font_data`; sections 8/10 reproduce (22/22 … 13/13 tables, CoreText reloads 5/6, EffectCraft accepts 6/6); 645 faces scanned, 32 `hvgl`. New bytes: `capture/review-documents-report.txt` 46,203 B sha `a4694a8f…`; differ from A1's 46,188 B sha `e05f2b64…` only in pid/timestamps/container UUIDs, one env line (`TERM`), and ms timings. A1's originals kept in `capture/a1-run/` |
| Separate from the app: `cargo clean` + full rebuild of `ecprobe` for `aarch64-apple-ios-sim` and `aarch64-apple-ios` + examples | ran | builds clean, both targets |
| Host-vs-Mac finding | re-ran `host-checks.sh` against the fresh report | byte-identical to A1: app's 372 paths == `find` on the Mac; `st_dev` 16777232 vs 16777239; Helvetica 2,369,592 B vs 1,195,996 B; `GeezaPro.ttc` hashes equal; runtime copy 8 subfolders 37/74/5/27/1/106/2/14, no top-level files, nothing deeper than one subfolder |
| No sandbox in the simulator (canary) | fresh run | section 7 reads `/tmp/craftbridge-v1/capture/sandbox-canary.txt` OK — the app reads a file outside its container, as A1 said |
| CoreText: 303 descriptors → 103 files, 8 folders incl. `PrivateFrameworks/FontServices.framework/CorePrivate`, all under the runtime root, all readable | fresh run section 8 | identical (103/103 readable, 0 FAIL; per-folder 29/42/1/11/1/4/14/1; `CTFontManagerCopyAvailableFontURLs` absent) |
| `CTFontManagerCopyAvailableFontURLs` unavailable, rest compile | `swiftc -typecheck` from the checked-in script | same single error; also verified by header reading that `CTFontManagerCopyAvailableFontFamilyNames`/`…PostScriptNames` are `CT_AVAILABLE(ios(10.0))` — added to RESULT §3 |
| Family counts agree with `UIFont.familyNames` | fresh run | 86 = 86 = 86, 0 missing |
| Table-rebuild route byte-for-byte + `add_font_data` accepts | fresh run section 10 | identical for all 6 samples, incl. PingFang face 20/32 (21/21 tables) that CoreText itself refuses to reload |
| `walk`: breadth first, `depth < 8`, dedups by canonical path, ttf/otf/ttc/otc, file roots skipped silently | read `fonts.rs:112-135` from the pinned rev via `git show`; matched against report behaviour (`[one font file]` → 0 faces) | claims all correct; line reference correct |
| 633 faces / 311 families via CoreText dirs; diff vs Fonts-root list = 22 PingFang-ish families lost, 7 Watch-only gained | re-ran `dirfamilies` twice on the Mac; checked the Watch files directly | byte-identical outputs (296 vs 311; exactly the 22 + 7). The 7 = `Watch/ADTTime.ttc` (6 hidden `… Time`) + `Watch/SukhumvitSet.ttc` (Sukhumvit Set) — confirmed by running `faces` on both files |
| PingFang `hvgl`-only; fallback results | re-ran `outline-formats.py` (identical: PingFangUI.ttc the only `hvgl` file) and both `fallback` spawns with absolute paths | byte-identical to A1's capture: `汉`(zh-Hans) → Inter/Regular (nothing covers it), `漢` → Hiragino Sans/W4, `한` → Apple SD Gothic Neo; `PingFang SC` parses, glyph 484, `has_char` false |
| `has_char` requires a drawable outline (glyf/CFF/CFF2/sbix/CBDT/COLR/EBDT); bundled set is Inter(5 styles)+JetBrains Mono+Noto Serif = 7 faces/3 families | read `fonts.rs` | correct; explains "faces before 7" and the 314 = 311 + 3 |
| Chinese fallback lists hold only PingFang among iOS families | grepped the full 311-family runtime list for all 18 Simplified/Traditional list members except PingFang | none present — claim holds (but see defect B1-3: the lists do not *start* with PingFang) |
| os_log truncation (~1 KB) | measured the A0 worker message in the log store | cut after ~0.8 KB at `DYL<…>`; "about 1 KB" is fair; current build logs one pointer line only — confirmed a fresh pointer line via `log show` for the new runs (pid 3357: "46203 bytes") |
| iPhoneos unsigned build links | re-ran after deleting `build/` | BUILD SUCCEEDED, arm64 binary, 6 `ecprobe_` symbols (an `appintentsmetadataprocessor` informational line appears in command-line builds; no compiler/linker warnings) |
| probe code quality (no duplicated logic) | read all six Swift files + `ecprobe/src/lib.rs` | clean: shared `list`/`describe`/`readHead` helpers, no re-implementations, section structure as RESULT §2 describes |

## 3. Defects found in round B1, and fixes

1. **RESULT §5 fallback commands were not reproducible** (real repro bug). The two `simctl spawn … fallback <list>` lines were transcribed with a *relative* list path (`capture/review-fontsroot-dir.txt`). `simctl spawn` runs the child with a different working directory, so the example silently read nothing and printed an all-Inter answer with `registered 0 faces` — a trap anyone following the doc would hit. A1's own capture is valid (its content shows the absolute paths and the correct 613/633-face runs); only the documented command was wrong. **Fix:** §5 now uses absolute paths with a comment saying why they matter. Verified: the absolute-path reruns are byte-identical to A1's capture.
2. **`ecprobe` examples swallowed missing/empty folder-list files** (`unwrap_or_default` → 0 dirs → misleading success). Same failure mode as above, at the source. **Fix:** `examples/fallback.rs` and `examples/dirfamilies.rs` now print a clear message and `exit(2)` when the `@list` file is missing/unreadable/empty. Verified (exit 2 on a bad path). Noted in RESULT §8.
3. **RESULT §3 said the Chinese fallback lists "start with `PingFang SC`/`PingFang TC`" — wrong.** They start with Windows families (`Microsoft YaHei`, `SimSun`; `Microsoft JhengHei`, `MingLiU`); PingFang is third in each list, and *is* the only family on those lists that iOS has (all 18 others grepped absent from the runtime family list). Reworded.
4. **RESULT §3 timing parenthetical said the 22–92 ms scans ran "on the Mac"** — they ran inside the app in the simulator. Reworded to "in the same in-app run".
5. **`FontProbe.swift` used `%d` for a Swift `Int` in the os_log pointer line** (format-specifier mismatch; works on arm64 little-endian by luck). **Fix:** `%zd`. The canonical captures in `capture/` are from a full clean re-run *with* the fixed source (46,203 B, `a4694a8f…`); round A1's originals are preserved byte-for-byte in `capture/a1-run/` (`e05f2b64…`, as REVIEW-A1.md records).
6. **Cosmetic, evidence provenance:** A1's `review-typecheck-ios.txt` header showed the scratch filename `avail.swift`, not the checked-in script. Regenerated from `review-scripts/coretext-availability.swift` — same single error, clearer provenance.
7. **Missing evidence strengthened:** "the only 32 of the runtime's 645 faces" had only a per-file capture. Added `capture/review-hvgl-face-counts.txt`: per face, exactly 32 faces carry `hvgl`, exactly 32 lack every drawable table, and the two sets are the same 32.
8. **Claim now backed with evidence:** "EffectCraft names the Mac copies of these fonts the same way" (the 5 Arabic-named families) — verified directly on Al Nile: Mac `Supplemental/Al Nile.ttc` and runtime `AppFonts/AlNile.ttc` yield identical EffectCraft face names (`النيل/عادي` etc.); captures `review-faces-mac-alnile.txt` / `review-faces-ios-alnile.txt`.
9. **Housekeeping:** round B1 re-run outputs are the `capture/review-*` files (canonical, documented in RESULT §8); `capture/a1-run/` holds A1's originals; intermediate per-check outputs (`hvgl-face-counts`, Al Nile faces, fallback/families re-runs) that confirmed byte-identity are either saved under their `review-*` names or documented as byte-identical duplicates of A1's.
10. **Not fixable, noted:** RESULT §8's claim that `capture/worker-sources/` hashes match "the build round's table" can no longer be independently checked (that table was overwritten when A1 rewrote RESULT.md); the copies themselves are kept and consistent with the changed files' history.

## 4. Small confirmations worth recording

- Section 4 extension math checks out: 204 ttf + 128 ttc + 38 otf + 2 extensionless = 372 (Mac), and 264 + 2 = 266 (runtime); `Supplemental` = 290 files, `Fonts/AppFonts` root files 82.
- `HelveLTMM`/`TimesLTMM` live in `<RT>/System/Library/Fonts/Core/` (the two extensionless files), so EffectCraft skips exactly 2 there — matches "264 loadable".
- No symlinks anywhere under the runtime `Fonts` tree (0 from `find -type l`).
- "Al Nile missing on the Mac family list" looked like a defect mid-review; it was a grep artifact (Arabic forms) — the font is present on the Mac at line 360 of the Mac family list and named identically by EffectCraft on both copies (defect B1-8's capture).
- Cargo.toml dependency is pinned to rev `103760584a60074c9aaeef686d2d05d89f08155e` and `Cargo.lock` agrees.

## 5. Agreement with the recommendation

**Agree.** Round B1 independently reproduced every measurement behind it:

- Take the font folders from CoreText's URLs: `CTFontCollectionCreateFromAvailableFonts(nil)` → `kCTFontURLAttribute` → distinct parent folders → `add_source(&DirectorySource { dirs })`. Reproduced: 633 faces / 263 files / 311 families in-app, all from runtime files, ~25 ms in the simulator, correct Arabic-named families included. `/System/Library/Fonts` is wrong (Mac's fonts in the simulator; misses PingFang on device because Apple keeps it in `CorePrivate`), file URLs are silently ignored by `walk`, and the common parent (`/System/Library`) would sweep the whole system library.
- The `CTFontCopyTable` → sfnt → `add_font_data` route works byte-exactly and EffectCraft accepts everything (even the rebuilt PingFang CoreText itself rejects), but costs full in-memory copies (352 MiB for all; Emoji 139 MB) and `add_font_data` does not dedupe — correct as a per-font device fallback only.
- **Chinese needs a bundled CJK font regardless of the folder choice**, because iOS 26's PingFang is `hvgl`-only and EffectCraft 10376058 cannot draw it (`has_char` false; `汉` falls back to Inter everywhere). Bundling Noto Sans SC/TC through `Folders.fonts` puts families on EffectCraft's own Chinese fallback lists. Agree this needs a plan decision.

## 6. Open items after round B1

- §7 device run on "SeaEl iPhone" — not run, device unavailable; V1's device half stays open.
- FX-3: decide hiding the 92 dot-named families in `list_fonts` presentation.
- The plan: choose the bundled CJK font(s) for Simplified/Traditional Chinese.
