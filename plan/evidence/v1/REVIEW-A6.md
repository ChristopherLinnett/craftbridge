# Review round A6 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round A6 reviewed two things. First, the device half: the orchestrator's additions to RESULT.md, checked against `capture/device-documents-report.txt` and the canonical simulator report. Second, round B6's three fixes and its device re-run, `capture/b6-check/`. It then re-read the whole of RESULT.md with the device half in it, fixed what it found, and ran a simulator check run into `capture/a6-check/`.

Work was alone. No font was downloaded, no GitHub request was made, no git state was changed, nothing was posted, and the simulator was left as found. The FX-3 decision itself (craftbridge#113) was not read, because that needs GitHub; RESULT's account of it was taken as given.

## 1. Environment and starting state

- **Simulator:** iPhone 17 Pro, booted, iOS 26.5. Its apps before and after the round: FontProbe and `com.example.orchestratorFlutter`. FontProbe was reinstalled by `run-sim.sh`; the other app was not touched.
- **Device:** at the start of this round `xcrun devicectl list devices` showed the acceptance iPhone as **`unavailable`** again. `devicectl device info apps` could not find it either. So this round did not re-run the device. Its device checks rest on the two device runs already captured: the canonical run and B6's `capture/b6-check/`. Nothing was sent to the phone.
- The canonical captures (`capture/review-*`, `sandbox-canary.txt`, `capture/device-*`) were hashed before the round. After it they were unchanged, apart from the one capture this round added (§4).

## 2. The device addition and B6's claims, checked

| Claim | How A6 checked | Result |
|---|---|---|
| §8: device report `45c9805e…`, 40,589 B; `run-device.sh` `39307445…` | `shasum`, `wc -c` | match |
| §7 row 1: a real folder, `realpath` unchanged, `Simulator : no` | device report lines 6–10 | holds |
| §7 rows 2, 3, 6: 8 subfolders, no top-level files, `Watch` among them, `Core/` 74 | device sections 2, 3, 4, 6. Section 4 gives 167 ttf, 74 ttc, 23 otf and 2 without extension, 266 in all, as in the simulator's runtime copy | holds |
| §7 row 7: canary 260, not 257 | device section 7: `NSCocoaErrorDomain code 260 … no such file`, underlying POSIX 2 | holds |
| §7's "line for line" sentence, as B6 fixed it | my own normalising diff of sections 8–10: `<RuntimeRoot>` removed, container paths and ms masked | the only differences are the simulator's `under the runtime root` line, its extra `dirs` scan of the runtime-root copy, and the `dirs = [/System/Library/Fonts]` counts (787/370/378 against 613/264/296). This is exactly B6's diff and the sentence as fixed. Section 10, file sizes included, is identical, so the iOS 26.6.1 device and the iOS 26.5 runtime hold the same font files |
| §7 rows 8–10 and the Verdict bullet: 103 files in 8 folders, `OK 103, FAIL 0`, 633/263/311, 314, 81/86 plus the 5 Arabic-named, the same fallback answers, Fonts root 613/264/296 without `CorePrivate` | device sections 8–10 | holds |
| §7: stdout is the same run with CRLF ends and devicectl's own lines | stripped `\r` and devicectl's lines (the timestamped ones, `Launched application…`, `Waiting for…`, `App terminated…`) from `device-stdout.txt` and compared with the Documents copy, for the canonical run and for B6's | byte-identical in both runs |
| `device-stderr.txt` holds the app's one os_log line | read it | one line: "40589 bytes", equal to the report size |
| B6 §4: its re-run differs from the canonical run only in instance values | `diff` of the two device reports | nine lines: timestamp, pid, `TERM`, XPC id, three timings, and the body size and SHA plus the READ-BACK count, which move with the timing digits. Every count is the same |
| B6 fixes 1–3 (the §7 sentence, the §4 "owner's call" wording, the scope line) | read the text against the evidence above | all three are in place and correct |

## 3. The simulator check run (`capture/a6-check/`)

Sources were unchanged, so this was not a clean build. With `CAPTURE_DIR=capture/a6-check` I ran `run-sim.sh` (exit 0) and `engine-checks.sh` (exit 0), and wrote the typecheck (exit 1, the one expected error) and devicectl captures there too.

Compared with `capture/`:
- **Byte-identical (11):** the folder lists, the family lists and their diff, the Arabic names, the Han fallback, the outline formats, host-checks, the typecheck and devicectl.
- **Instance values only:** the report and stdout, stderr and os_log.
- **New:** `review-coretext-file-families.txt` (§4), promoted to `capture/`.

## 4. Defects found in round A6, and fixes

1. **major — RESULT §4 recorded the owner's `list_fonts` decision next to an acceptance check that the obvious way of implementing the decision fails.**
   - **The decision and the check:** `list_fonts` is to show "only what CoreText lists plus the bundled fonts". The acceptance check requires all 86 `UIFont.familyNames` families, 5 of them under the Arabic names EffectCraft gives them.
   - **A name filter fails it.** Matching EffectCraft's family names against CoreText's keeps only 81: Al Nile, Damascus, Farah, Geeza Pro and Mishafi drop out, because EffectCraft names them in Arabic (section 9).
   - **A file filter shows more.** Keeping every face of CoreText's 103 files keeps those 5. But EffectCraft reads 123 families from those files: the 86, 30 dot-named ones and 7 visible families that UIKit does not list (six Hiragino Pro/Std cuts and Noto Sans Zawgyi, all among section 9's 141 non-UIKit families).

   Followed literally, RESULT would have led FX-3 to ship a font menu without five of the iPhone's own families, or to fail its own check. **Fix:**
   - `engine-checks.sh` now writes `capture/review-coretext-file-families.txt`: the families EffectCraft reads from the 103 files section 8 names, with the count and the dot-named count. I cross-checked it against a hand computation over the same files.
   - §4 has a new sub-bullet with both outcomes: the name filter gives 81, and the file filter gives 123, or 93 with dot-named families hidden. The 93 are UIKit's 86 plus those 7.
   - §8 has a row for the new capture and the new `engine-checks.sh` hash.
2. **minor — two places in §4 were left from before the decision.**
   - The AD2 sub-bullet still said what `list_fonts` shows "is FX-3's call (open items below)", but RESULT has no open-items section, and the owner has decided.
   - The decided font subsection still headed its file table "Proposed files".

   **Fix:** the first now points at the decision in the Chinese bullet and at the new filter bullet; the second reads "The files".
3. **minor — §7 stated an unbacked error string.** It said that without a signed-in account "the build fails with `No Accounts`". No capture holds that output. **Fix:** §7 now quotes `xcodebuild -help` instead: `-allowProvisioningUpdates` "Requires a developer account to have been added in Xcode's Accounts settings". It also names the menu as Xcode → Settings → Accounts.

## 5. Considered, not defects

- **Device timing.** On the device, `add_source` + `families()` took 50 ms, and 47 ms in B6's re-run. §3 only says "a phone will be slower", and that holds. Timings are instance values under §8.
- **Device re-checks in §1.** The Environment row lists the rounds that found the device `unavailable` before 2026-11-13 only up to B3. That is a per-round record, and it stays true as written.
- **Device check runs and the copy rule.** §8's method paragraph names only `capture/review-*` and the canary in its copy rule. The device captures (`capture/device-*`) are canonical too. A round that promotes a device capture should copy the old one first. B6 and A6 promoted none, so nothing was lost; this is a gap in wording for a later round, not a content error.

## 6. Agreement with the recommendation

**Agree, now confirmed on the device.**
- **The folders:** AD2 takes the parent folders of CoreText's font files and passes them to `add_source(&DirectorySource { dirs })`. On the iPhone, the app reads all 103 files (`OK 103, FAIL 0`), and those folders give the same 633 faces in 311 families as in the simulator.
- **Hard-coding is still wrong:** a hard-coded `/System/Library/Fonts` misses `CorePrivate` on the device too (613/264/296).
- **No table fallback:** rebuilding fonts from CoreText's tables is not needed.
- **Chinese:** it needs the bundled Noto Sans SC/TC the owner chose, because the device's fallback answers are the simulator's: 汉 → Inter, 漢 → Hiragino Sans.
- **New for FX-3:** the `list_fonts` filter must keep the 5 Arabic-named families (§4, defect 1).

## 7. Open items

- **FX-3's `list_fonts` filter:** match by family name, by file, or something else, given §4's numbers. It is FX-3's to choose, within the owner's decision.
- **The bundled fonts:** the family names inside the two Noto files, and the fallback drawing with them, are unverified until FX-3 fetches the files (§4).
- **Device re-runs:** possible only when the acceptance iPhone is connected again; it was `unavailable` during this round.

## 8. Outside the workspace

- Read-only: `simctl` and `devicectl` queries, and the SDK used by the build.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds.
- Scratch `a6-scratch/` held the hashes, the normalised sections, the hand-computed family list and the edit scripts, and was deleted at the end. `capture/a6-check/` is kept as this round's check run.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 3 defects fixed (1 major, 2 minor); recommendation agreed and confirmed on the device; device not re-run this round (unavailable).**
