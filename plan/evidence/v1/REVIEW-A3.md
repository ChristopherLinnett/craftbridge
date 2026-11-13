# Review round A3 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round A3 re-ran every claim and fix of round B2, reviewed the whole investigation again (RESULT.md, the Swift probe, the `ecprobe` library and examples, the scripts and the captures), fixed what was wrong in place, and re-ran §5 from a clean build. EffectCraft was read only at the pinned rev, through `git show 103760584a60074c9aaeef686d2d05d89f08155e:<path>` in `~/dev/artcraft-dev/effectcraft/repository` (never checked out or built there). Work was alone: no agents, no workers. No font was downloaded.

## 1. Environment re-check

- Simulator iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`, booted, iOS 26.5 (23F77) the only runtime; Xcode 26.6 (17F113). Third-party apps before and after the round: `com.craftbridge.FontProbe` and `com.example.orchestratorFlutter`. FontProbe stayed installed (`run-sim.sh` reinstalls it); the other app was not touched; the simulator was left booted.
- **Device: `xcrun devicectl list devices` shows "SeaEl iPhone" (iPhone 12 Pro, iPhone13,3) `unavailable`.** `capture/review-devicectl.txt` was made again, byte-identical. Nothing was installed or run on a device.

## 2. Round B2's claims and fixes, re-run

| B2 claim or fix | How A3 checked | Result |
|---|---|---|
| §8 hashes | `shasum -a 256` on every file §8 lists | all 23 matched before A3 changed anything |
| `capture/a2-run/` holds A2's captures; `capture/b1-run/` unchanged | hashed against the values A2 recorded (`4c544c06…`, `cde4825c…`, `99d17850…`, `34c44857…` and others; B1's report `a4694a8f…`) | both intact |
| B2's report differs from A2's only in instance values | `diff` of the two reports | true: pid, timestamps, container paths, XPC id, one `TERM = dumb` line, timings (B2: scans 93/102/14 ms, `add_source` 22 ms). Every count identical |
| fix 1: `noto-cjk-sizes.py` reproduces the noto capture | ran it, first to a scratch file | **the first run failed: GitHub answered HTTP 403, rate limit exceeded** (`x-ratelimit-limit: 60`, `remaining: 0`). With §5 step 6 as written (`> capture/review-noto-cjk-sizes.txt`), the shell would have emptied the capture before the script failed (defect 1). After the limit reset, the fixed script reproduced the capture: only the fetch-time line differs from B2's and A2's |
| fix 1, the script itself | read it | its docstring says the capture "is written by review-scripts/noto-cjk-sizes.sh", which does not exist (defect 2), and its `listing()` repeats `github-dir-sizes.py`'s format, which no command used any more (defect 3) |
| fix 2: RESULT cites B2's run | read §3, §5, §8 against the kept reports | the "15 to 25 ms over seven in-app runs (A1, B1 twice, A2 twice, B2 twice)" is backed by no kept report and miscounts A2's three runs (defect 6); §8 says "the earlier runs' shells did not" export `TERM`, but B1's report has the same `TERM = dumb` line (defect 7); §5 says A2 "ran steps 1 to 6", but A2's step 6 was a single `curl` listing (defect 8) |
| fix 3: §4 "everything under it eight folders deep" | read `walk` (`fonts.rs:112-135`): files of the root and of folders down to depth 8 | right, but the sentence read "the 8-level walk would sweep everything under it eight folders deep". Tidied |
| fix 4: the memory bullet | checked each number against the report and the files | wrong reasoning (defect 5): it says the table path costs "one file's worth of bytes per font", so the full set "costs 352 MiB". The copy is per face, and a collection's faces share tables: PingFang SC's face 20 rebuilds to 58,965,676 B of a 61,080,840 B file that holds 32 faces. Its "438 MiB" for the walk's 263 files had no capture in the workspace (only in `/tmp/b2-logs/`), and the folder path never loads those bytes up front anyway |
| `run-sim.sh` stops on a planted compile error | planted `let a3PlantedError: Int = "not an int"` in `FontProbe.swift` (backup kept, restored, SHA-256 `58a72c5c…` verified), ran the script, once before and once after A3's fix | exit 1 at the build step; nothing installed; the report untouched. Before the fix it printed only `** BUILD FAILED **`, with no compiler message and no log (defect 4); after it, the `error:` line and `build/xcodebuild-iphonesimulator.log` |
| `simctl uninstall` of an absent app exits 0 | `xcrun simctl uninstall booted com.craftbridge.NoSuchApp` | exit 0 (tested on an id that was never installed, so FontProbe stayed in place) |
| a foreign `CARGO_TARGET_DIR` does not leak | final run with `CARGO_TARGET_DIR=/tmp/craftbridge-v1/a3-foreign-target` exported | that folder was never created |
| cited URLs | `curl` status of the three Apple pages, the noto-cjk repo and its `LICENSE` | all 200 |

## 3. Defects found in round A3, and fixes

1. **major — §5 step 6 could empty the capture it was meant to reproduce.** `python3 -I review-scripts/noto-cjk-sizes.py > capture/review-noto-cjk-sizes.txt` truncates the file before the script runs, and the script stopped with a traceback on the first refused request. GitHub allows 60 unauthenticated API requests an hour per address, and this round's first run hit that limit (HTTP 403). **Fix:** the script takes the output path, builds the whole text first, writes a temporary file and renames it only when every request succeeded. A refused request prints the URL, the status and a rate-limit hint, and leaves the capture as it was (tested while the limit was in force: exit 1, capture hash unchanged). It also uses a 30 s timeout. §5 step 6 now passes the path.
2. **minor — `noto-cjk-sizes.py`'s docstring named a script that does not exist** (`review-scripts/noto-cjk-sizes.sh`) and contradicted its own usage line. **Fix:** rewritten docstring; running the script without the argument prints it.
3. **minor — duplicated code:** `noto-cjk-sizes.py` repeated the listing format of `github-dir-sizes.py`, which no command in RESULT used any more. **Fix:** `github-dir-sizes.py` deleted; `noto-cjk-sizes.py` is the one definition; §8 updated.
4. **minor — `run-sim.sh` hid compiler errors.** It piped xcodebuild through `grep '^\*\* BUILD'`, so a failed build showed `** BUILD FAILED **` and nothing else, and the log was gone. **Fix:** the full log goes to `build/xcodebuild-iphonesimulator.log`; on failure the script prints its `error:` lines and the log path, then exits 1. Checked with the planted error: `FontProbe.swift:295:27: error: cannot convert value of type 'String' to specified type 'Int'`.
5. **minor — RESULT §3's memory bullet (B2's fix 4) was wrong and partly unbacked.**
   - It priced the table path at one file per font. The copy is per face, and each face carries the tables its collection shares.
   - It cited the walk's 263 files at 438 MiB, a figure measured only in B2's scratch folder. That figure also does not apply: `DirectorySource` reads name tables when it scans, reads a file on first use, and shares the bytes among a collection's faces (`fonts.rs:72`, `:40`, `:209-212`).
   - It wrote "§8" and "§10" for report sections, while RESULT's own §8 is the file list.

   **Fix:** section 10's `file comparison` line now gives each file's size (probe change in `FontProbeCoreText.swift`). The bullet now says what was measured: PingFang SC's face 20 is 58,965,676 B of the 61,080,840 B `PingFangUI.ttc` (32 faces), and Apple Color Emoji's face 0 is 139,052,436 B of 139,284,540 B. It also says that 352 MiB is not a bound on the table path, and adds the folder path's lazy, shared loading with source lines.
6. **minor — RESULT §3's timing range rested on runs whose reports are gone**, and the run count was wrong: A2 made three in-app runs, not two. **Fix:** the range now comes from the kept reports (`a1-run`, `b1-run`, `a2-run`, this run): 16 to 25 ms for `add_source` + `families()`, 13 to 126 ms for the folder-list scans. B2's recorded 22 ms and its unkept 15 ms are named as such.
7. **minor — RESULT §8 said only B2's shell exported `TERM`.** B1's report carries the same `TERM = dumb` line. **Fix:** §8 says which runs have it (B1, B2) and which do not (A1, A2, A3).
8. **minor — RESULT §5 said rounds A2 and B2 "each ran steps 1 to 6".** A2's step 6 was a one-listing `curl`, not the capture script. **Fix:** the paragraph now says what each round ran, including this round's planted-error and foreign-`CARGO_TARGET_DIR` checks.

Also tidied, not counted: the §4 sentence on the common parent ("the 8-level walk would sweep everything under it eight folders deep") now says it once. The scope line and the environment table now name round A3.

### A slip in this round

Earlier rounds kept the previous round's captures in `capture/<round>-run/` before re-running. Round A3 copied B2's noto capture to `capture/b2-run/`, but only after its own `run-sim.sh` had replaced B2's report, stdout, stderr and os_log captures. Those four B2 files are lost. What they held is recorded:
- **What was in them:** the `diff` in §2 above: instance values, one `TERM` line and the timings. Their SHA-256 (`82546a52…`, 52,598 B) is in REVIEW-B2.md.
- **B2's other captures:** byte-identical to `capture/a2-run/`.

RESULT §8 says so.

### Final clean run

1. Removed `rm -rf build ecprobe/target`, then exported `CARGO_TARGET_DIR=/tmp/craftbridge-v1/a3-foreign-target`.
2. `run-sim.sh` with the planted error: exit 1 with the error line. Source restored (`58a72c5c…`).
3. `run-sim.sh`: exit 0. Report 52,681 B, stdout == Documents copy, SHA-256 `0e8b07f0…43bca`; os_log line "52681 bytes".
4. `engine-checks.sh`: exit 0.
5. iphoneos unsigned build: BUILD SUCCEEDED, 6 `T _ecprobe_` symbols.
6. Typecheck: exit 1 with the one expected error.
7. `review-devicectl.txt` and, after the rate limit reset, `noto-cjk-sizes.py capture/review-noto-cjk-sizes.txt` (exit 0; only the fetch-time line differs).
8. The foreign target folder was never created.

Every count reproduced. Byte-identical to round A2's captures:
- `review-host-checks.txt`, both folder lists and both family lists, the family diff;
- the Arabic names, the Han fallback, the outline formats;
- the typecheck and devicectl captures.

The report differs from A2's only in instance values, timings (scans 126/126/20 ms, `add_source` 22 ms) and the new file sizes in section 10.

## 4. Agreement with the recommendation

**Agree, unchanged.** AD2 on iOS should take the parent folders of the files CoreText reports through `CTFontCollectionCreateFromAvailableFonts` + `kCTFontURLAttribute` and pass them to `add_source(&DirectorySource { dirs })`:
- **Rejected alternatives:** a hard-coded `/System/Library/Fonts` gives the Mac's fonts in the simulator and misses `CorePrivate`. `walk` ignores file URLs, and the common parent is far too wide.
- **The extra families:** the folders register 263 files where CoreText offers 103. That helps fallback and leaves a display question for FX-3.
- **Chinese:** it still needs a bundled CJK font. iOS draws Chinese with PingFang, whose `hvgl` outlines skrifa 0.44.0 cannot read.
- **Fallback for a device that refuses file reads:** this round's measurement adds weight. The table path copies a collection's shared tables once per face, so it only fits a few fonts.

## 5. Open items

- **Device half of V1:** "SeaEl iPhone" was `unavailable`. Nothing ran on a device; §7's recipe stands.
- **FX-3, what `list_fonts` shows** (not decided): the 92 dot-named and 133 extra visible families; the options are unchanged from A2.
- **The bundled CJK font** (owner's call): §4's noto-cjk proposal stands; its sources were re-fetched this round and match. The family names inside the files and the fallback drawing with them stay unverified until someone downloads the files.

## 6. Outside the workspace

- Read-only: `git show` in the EffectCraft clone; keyless `curl` and Python `urllib` requests to the GitHub API, raw GitHub files and the Apple documentation pages.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds.
- Every scratch file stayed in `/tmp/craftbridge-v1` (`build/`, and the source backup, deleted after the restore was verified). Nothing was written elsewhere.
- No git repository's state was changed, nothing was posted, and `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 8 defects fixed (1 major, 7 minor); recommendation agreed; device unavailable, not run.**
