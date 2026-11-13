# Review round A5 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round A5 checked round B4's claims and fixes by running them, and reviewed the whole investigation again: RESULT.md, the Swift probe, `ecprobe`, the scripts and the captures. It fixed what it found and applied the coordinator's new rule for run and check folders. It followed RESULT §8's capture method. The canonical captures are unchanged since round A4, and `capture/a4-run/` already holds them byte for byte (checked), so A5 made no new copy and ran into `capture/a5-check/`.

EffectCraft was not re-read beyond what RESULT cites; nothing in this round turned on it. Work was alone. No font was downloaded, no GitHub request was made, no git state was changed, nothing was posted, and no physical device was used.

## 1. Environment and starting state

- The simulator is iPhone 17 Pro `E33474EA-1897-4EF8-9DFB-DD3FFA178C8D`: booted, iOS 26.5 (23F77). Third-party apps before and after the round are `com.craftbridge.FontProbe` and `com.example.orchestratorFlutter`. FontProbe stayed installed (`run-sim.sh` reinstalls it), the other app was not touched, and the simulator was left booted.
- **Device: "SeaEl iPhone" (iPhone 12 Pro) is `unavailable`.** The check capture is byte-identical to the canonical one.
- I hashed the 17 canonical files (`capture/review-*`, `sandbox-canary.txt`) before the round and checked them again after it: all 17 unchanged. All 22 SHA-256 values in §8 matched before any edit; this round changed none of those files.

## 2. Round B4's claims and fixes, checked

| B4 claim or fix | How A5 checked | Result |
|---|---|---|
| fix 1: §8 rows for `capture/a4-run/` and `capture/b4-check/` | read §8; compared `a4-run/` with `capture/` | the rows were there, and `a4-run/` equals the canonical set. Both rows are now folded into the pattern line (§4 below), as the new rule asks |
| fix 2: §5's preamble says step 3 only builds and prints | read §5; ran step 3 | true: step 3 writes into `build/` and prints `** BUILD SUCCEEDED **` and the 6 `T _ecprobe_` symbols; nothing goes into the capture folder |
| a check run leaves `capture/` untouched | `shasum -a 256 -c` over the 17 canonical files after the check run | all OK |
| `FontProbe.xcodeproj` has no build phase that calls cargo | `grep -c PBXShellScriptBuildPhase` | 0 |
| a foreign `CARGO_TARGET_DIR` is never used | check run with `CARGO_TARGET_DIR=/tmp/craftbridge-v1/a5-foreign-target` | that folder was never created |
| B4's other re-derivations (per-folder table, 22/7 families, 102 + 31 = 133, the extensionless pair, outline formats, Noto sizes, citations) | the check run reproduced the captures they rest on (below) | hold |

## 3. The check run (`capture/a5-check/`)

I deleted `build/` and `ecprobe/target/`, then ran §5 steps 1 to 5 with `CAPTURE_DIR=capture/a5-check` and the foreign `CARGO_TARGET_DIR`. Step 6 was skipped, since no GitHub requests are allowed.
- `run-sim.sh`: exit 0; stdout == Documents copy; 52,682 B.
- `engine-checks.sh`: exit 0.
- iphoneos build: `BUILD SUCCEEDED`, 6 `T _ecprobe_` symbols.
- Typecheck: exit 1 with the one expected error.
- devicectl: `unavailable`.

Compared with `capture/`:
- **Byte-identical (11):** both folder lists, both family lists and their diff, the Arabic face names, the Han fallback, the outline formats, host-checks, the typecheck and devicectl.
- **Instance values only:**
  - The report and stdout differ in pid, timestamps, container paths, the XPC id and ms timings, plus the absent `TERM` line, which accounts for the 52,682 B against 52,697 B.
  - stderr and os_log differ in timestamps, pid and that byte count.
  - Every count is the same: 787/370/378, 613/264/296, 633/263/311, 314 families, 303/103/8, 645/32, 86 = 86 = 86, the per-folder table and the fallback answers.

So, by §8's rule, nothing is promoted, and the canonical report and RESULT's run record (round B3's run, 52,697 B, `f2844cb0…`) stay as they are.

This run was the slowest so far: `add_source` + `families()` took 30 ms, and the three folder-list scans took 243 / 128 / 29 ms. The timings in all eight distinct runs whose reports are kept as of this round:

| Report | Scans (Mac root / runtime root / CoreText folders) | `add_source` + `families()` |
|---|---|---|
| `capture/a1-run/` | 79 / 92 / 22 ms | 25 ms |
| `capture/b1-run/` | 16 / 14 / 13 ms | 16 ms |
| `capture/a2-run/` | 15 / 14 / 13 ms | 16 ms |
| `capture/a3-run/` | 126 / 126 / 20 ms | 22 ms |
| `capture/` (round B3, canonical) | 111 / 114 / 14 ms | 17 ms |
| `capture/a4-check/` | 103 / 107 / 14 ms | 16 ms |
| `capture/b4-check/` | 103 / 103 / 13 ms | 16 ms |
| `capture/a5-check/` | 243 / 128 / 29 ms | 30 ms |

(`capture/b3-run/` and `capture/a4-run/` hold copies of the canonical report, so they add no run.)

## 4. The coordinator's change of method (not a defect)

RESULT §8 now has one pattern line in place of a row per run or check folder:
- `capture/<round>-run/` holds the canonical captures as that round left them, copied by a later round before it changed them.
- `capture/<round>-check/` holds that round's check run.
- The REVIEW file of the round that made a folder describes it, and a folder without a row is not a defect.

Rows stay only for the folders whose content RESULT cites: `a1-run/`, `b1-run/`, `a2-run/` and `a3-run/`, which §8's report and host-checks rows compare against. The rows for `b2-run/`, `b3-run/`, `a4-run/`, `a4-check/` and `b4-check/` are gone; REVIEW-A3, A4 and B4 describe those folders. Two §8 rows that pointed at `capture/a4-check/` now point at REVIEW-A4.md instead.

The method paragraph also now says to copy the canonical captures only when they changed since the last copy, into `capture/<round that left them>-run/`. That is what this round did by making no copy.

## 5. Defects found in round A5, and fixes

1. **minor — RESULT §3's timing sentence was contradicted by a kept report.** It said `add_source` took "16 to 25 ms in each run whose report is kept", with the scans at "13 to 126 ms". Under §8's method the check folders are kept reports, and this round's check run took 30 ms, with the scans up to 243 ms. A sentence over "each kept report" will break again with the next slow run. **Fix:** it now gives the canonical run's 17 ms and the range over the eight runs kept as of round A5 (16 to 30 ms; scans 13 to 243 ms; the table above). It also says timings vary with the Mac's load and are instance values under §8. The numbers RESULT's findings rest on are unchanged.
2. **minor — RESULT §4 said "Not verified in this round".** That wording dates from round A2, and RESULT is no longer re-pointed every round, so "this round" names no round. **Fix:** "Not verified yet".

No other defect was found. Every figure, citation and behaviour claim that the check run or a reading could test held, including B4's two fixes.

## 6. Agreement with the recommendation

**Agree, unchanged.** AD2 on iOS should take the parent folders of the files CoreText reports through `CTFontCollectionCreateFromAvailableFonts` + `kCTFontURLAttribute` and pass them to `add_source(&DirectorySource { dirs })`. Three alternatives are wrong:
- `/System/Library/Fonts` gives the Mac's fonts in the simulator and misses `CorePrivate`.
- File URLs are ignored by `walk`.
- The common parent of those folders is far too wide.

Those folders register 263 files where CoreText offers 103, which is a display question for FX-3. Chinese still needs a bundled CJK font, because PingFang's `hvgl` outlines are unreadable to skrifa 0.44.0.

## 7. Open items

- **Device half of V1:** "SeaEl iPhone" is still `unavailable`; §7's recipe stands.
- **FX-3, what `list_fonts` shows:** the 92 dot-named and 133 extra visible families. Not decided.
- **The bundled CJK font:** the owner's call. §4's noto-cjk proposal stands. The family names inside the files and the fallback drawing with them stay unverified until the files are downloaded.

## 8. Outside the workspace

- Read-only use outside the workspace: `simctl` and `devicectl` queries, and the SDK used by the builds.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds.
- Scratch stayed inside `/tmp/craftbridge-v1`: `a5-scratch/` held the canonical hashes, the run log and the edit script, and was deleted at the end. `a5-foreign-target` was never created.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 2 defects fixed (0 major, 2 minor); recommendation agreed; device unavailable, not run.**
