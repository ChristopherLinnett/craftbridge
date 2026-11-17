# Review round B6 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B6 reviewed the device addition in full: every device claim in RESULT.md (the last Verdict bullet, the Environment row, the §4 changes, the §7 table, its prose and recipe, the §8 rows) against `capture/device-documents-report.txt` line by line and against the canonical simulator report; the whole document was re-read with them; the §7 "line for line" sentence was proved with a normalising diff (§3 below); and the device recipe was re-run into `capture/b6-check/` to prove reproducibility (§4). The fixes are §6.

Work was alone; no other agent or worker was started. On the iPhone only FontProbe (`com.craftbridge.FontProbe`) ran, through `review-scripts/run-device.sh`; nothing else was installed, no device setting was changed, and FontProbe was left installed. The booted simulator was never touched. No git command that changes state was run, nothing was posted, no font downloaded, no GitHub request made. The pinned EffectCraft rev did not have to be read: no claim under review this round turned on it.

## 1. Environment and starting state

- Device connected: `xcrun devicectl list devices` (run for this review, output not written into `capture/`) shows the acceptance iPhone, identifier `<device>`, state `available (paired)`, model `iPhone 12 Pro (iPhone13,3)` — as RESULT's Environment row and §7 header say.
- Hashes verified before touching anything and re-verified after the round: device report `45c9805e…` (40,589 B — §8's recorded SHA and size), canonical simulator report `f2844cb0…`, `run-device.sh` `39307445…`, `review-host-checks.txt` `20cf25df…`, and every other `capture/review-*` file and `sandbox-canary.txt`. All unchanged after the round: the check run wrote only into `$CAPTURE_DIR`.
- No source changed since the canonical device run, so (per the round brief) no rebuild was needed; the installed app was re-used.

## 2. The device claims, cell by cell

| Claim | Where checked | Result |
|---|---|---|
| §7 row 1: real folder on device, `realpath` unchanged, `Simulator : no` | device report lines 6, 9–10 | holds (`realpath(/System/Library/Fonts) = /System/Library/Fonts`; lstat directory; `Simulator : no (IPHONE_SIMULATOR_ROOT unset)`) |
| §7 row "2, 3, 6": 8 subfolders, no top-level files, `Watch` among them; `Core/` 74 files | §2 "OK: 8 entries = 0 files + 8 subfolders" (Watch listed); §6 "OK: 74 files, 0 folders"; §3 subfolder counts sum 266, §4 totals 264 sfnt + 2 nameless | holds; the simulator column ("Mac: 82 files plus Supplemental") matches the canonical report §2/§3 |
| §7 row 7: canary 260, not 257 | "NSCocoaErrorDomain code 260 … no such file", underlying POSIX 2 | holds |
| §7 row 8: 103 files, same 8 folders same file names, `OK 103, FAIL 0`, 86 = 86 = 86 differ in 0 | §8 lines 335–352; identical file lists and folder counts (29/42/1/11/1/4/14/1) proven by the §3 diff | holds |
| §7 row 9 (CoreText folders): 633 faces, 263 files, 311 families; per folder the same counts as §3; `families()` 314 (faces before 7); 81/86 English + 5 Arabic under their Arabic names; fallback 汉 → Inter/Regular, 漢 (zh-Hant) → Hiragino Sans/W4, 한 → Apple SD Gothic Neo/Regular | lines 372, 376–383 (all eight per-folder triples equal §3's table), 384, 390–397, 386–389 | holds |
| §7 row 9 (Fonts root): 613 / 264 / 296, still misses `CorePrivate` (PingFang); simulator 787 / 370 / 378 | device line 371; simulator line 498; §2 shows no `CorePrivate` under `Fonts/`, so a walk of that root cannot reach it | holds |
| §7 row 10: same 6 samples, all accepted by `add_font_data` | §10; the §3 diff shows section 10 byte-identical to the simulator report | holds |
| §7 header: READ-BACK line checks the copy; stdout is the same run with CRLF ends and devicectl's own lines | line 738 READ-BACK OK; `device-stdout.txt`: 738 CRLF report lines + 6 LF devicectl lines; body equal to the Documents copy after `tr -d '\r'` and dropping devicectl's lines | holds |
| Verdict bullet: 103 files / same 8 folders / `OK 103, FAIL 0`; 633 from 263 in 311 as in the simulator; all 86 `UIFont.familyNames` in `families()`; fallback answers the simulator's; device Fonts root 613/264/296, misses `CorePrivate` | as rows above; "all 86" = 81 English + the 5 Arabic-named families, each present in `families()` under its Arabic name (lines 708–712) | holds |
| Environment row: iPhone 12 Pro (iPhone13,3), iOS 26.6.1 (23G83), run 2026-11-13; `unavailable` only before | report line 3; `devicectl list devices`; timestamp 2026-11-13T06:11:01Z; `capture/review-devicectl.txt` is the historical capture the row cites in past tense | holds |
| §4: "V1, device half: confirmed"; AD2 sentence "the device run (§7) shows the sandbox lets the app read those files"; rebuilt-tables fallback "not needed"; Chinese bullet "the device's fallback answers are the simulator's" | `OK 103, FAIL 0` (reads); 633/263/311 (same as simulator); §10 samples are the fallback that is no longer needed; fallback lines identical to the simulator's (proven by the §3 diff) | holds |
| §8 rows: script SHA `39307445…`; device report `45c9805e…`, 40,589 B; stdout/stderr/copy descriptions | `shasum` matches; stderr is the one os_log line ("40589 bytes", equal to the file size); copy file is the copy command's output | holds |

No line of RESULT still said the device half was open or that the device was currently unavailable (line 22 says `unavailable` only of the earlier rounds, in past tense — true, `review-devicectl.txt` shows it).

## 3. The normalising diff (the §7 "line for line" sentence)

Sections 8–10 were extracted from both reports (`sed -n '/^--- 8\./,/^--- 11\./p'`; device 400 lines, simulator 402) and normalised: simulator `<RuntimeRoot>` prefixes removed, each side's app container path replaced by `<container>`, every `(N ms)` masked. The full normalised diff (`diff -u`) is these two hunks and nothing else:

```diff
--- sim-8-10.norm
+++ dev-8-10.norm
@@ -10,7 +10,6 @@
     4 in /System/Library/Fonts/UnicodeSupport: NotoSansSyriac-Regular.ttf, STIXTwoMath.otf, STIXTwoText-Italic.ttf, STIXTwoText.ttf
     14 in /System/Library/Fonts/WebFonts: Arial.ttf, ArialBold.ttf, ArialBoldItalic.ttf, ArialItalic.ttf, ArialRoundedMTBold.ttf, Georgia.ttf, GeorgiaBold.ttf, GeorgiaBoldItalic.ttf, GeorgiaItalic.ttf, Impact.ttf, Verdana.ttf, VerdanaBold.ttf, VerdanaBoldItalic.ttf, VerdanaItalic.ttf
     1 in /System/Library/PrivateFrameworks/FontServices.framework/CorePrivate: PingFangUI.ttc
-  under the runtime root: 103; under the Mac's /System/Library/Fonts: 0; elsewhere: 0
   read first 4 bytes of every file: OK 103, FAIL 0; total size of the readable files 352 MiB
     55 x hex [74 74 63 66] ascii "ttcf"
     47 x hex [00 01 00 00] ascii "...."
@@ -36,8 +35,7 @@
     <container>/.local/share/fonts (exists: false)
     <container>/.fonts (exists: false)
   DirectorySource { dirs }.faces() (lists, registers nothing):
-    dirs = [/System/Library/Fonts]: 787 faces, 370 files, 378 families (<ms>)
-    dirs = [/System/Library/Fonts] (the iOS copy; no font files at its top level): 613 faces, 264 files, 296 families (<ms>)
+    dirs = [/System/Library/Fonts]: 613 faces, 264 files, 296 families (<ms>)
     dirs = CoreText's 8 font folders: 633 faces, 263 files, 311 families (<ms>)
     dirs = [one font file, /System/Library/Fonts/AppFonts/AcademyEngraved.ttf]: 0 faces, 0 files, 0 families (<ms>)
     (their common parent /System/Library not walked: it is not inside a Fonts folder)
```

So after normalisation the two reports differ only by two simulator-only lines — section 8's `under the runtime root: 103; …` count line and section 9's `dirs` scan of the runtime-root copy (the simulator walks four roots where the device walks three, because on the device `/System/Library/Fonts` *is* that copy) — plus the counts of the matched `dirs = [/System/Library/Fonts]` line (the Mac's 787 / 370 / 378 against the iPhone's 613 / 264 / 296; the device's line matches the simulator's runtime-copy scan count for count). Everything else, section 10's rebuild sizes included, is identical line for line. The original RESULT sentence excepted only "the simulator's runtime-root count line" — one line short of what the diff shows; fixed in §6.

## 4. The device re-run (`capture/b6-check/`)

`CAPTURE_DIR=capture/b6-check review-scripts/run-device.sh <device>` exited 0. Against the canonical report the diff is nine lines, every one an instance value: timestamp; pid 540 → 545; XPC service id `[e964]` → `[93e1]`; `TERM` vt100 → dumb (the launching shell's value, §8's instance-value class); three scan timings (139 → 46, 2 → 1, 50 → 47 ms); and section 11's body byte count and SHA plus the matching READ-BACK count, which move with the timing digits (40,589 → 40,587 B, two digits fewer). Every count and finding is equal: 613 / 264 / 296, 633 / 263 / 311, all eight per-folder triples, 314, 81/86, the fallback lines, 86 = 86 = 86, `OK 103, FAIL 0`, the magic-number split, section 10 in full. Nothing was promoted per §8's re-run rule; `capture/b6-check/` keeps this round's evidence (covered by §8's pattern row). The b6-check stdout also matches its own Documents copy, re-proving §7's stdout sentence on a fresh run. Canonical `capture/` unchanged before and after (hash re-checked).

## 5. The device script

`shellcheck` 0.11.0 (Homebrew): clean, no output. Writes everything into `${CAPTURE_DIR:-capture}` (line 8: stdout, stderr, the report copy), with `set -u -o pipefail`, an unbound-`$1` guard, a 600 s READ-BACK deadline and an early-exit path if devicectl dies first. Its behaviour matches §7's recipe text exactly: `--console` launch, wait for READ-BACK, SIGINT to devicectl — §7's quote from `devicectl device process launch --help` ("Catchable signals sent to devicectl are forwarded to the app.") is verbatim — then `devicectl device copy from` the app's Documents container to `$C/device-documents-report.txt`. §8's SHA `39307445…` matches the file. The §4 re-run proves it works end to end.

## 6. Defects found and fixed

1. **§7, the "line for line" sentence** (RESULT line 184): its exception list ("paths, process and container ids, timings and the simulator's runtime-root count line") missed the simulator's extra `dirs` scan of the runtime-root copy, so the sentence was strictly false. Rewritten to except both simulator-only lines and to name the only count difference explicitly with both triples (787 / 370 / 378 vs 613 / 264 / 296), as proved by the §3 diff.
2. **§4, "Not verified yet" bullet**: "Both need the files downloaded, which is the owner's call" contradicted the Chinese bullet above it ("The owner chose the proposal below … both recorded on FX-3"). Rewritten: the decision is recorded on FX-3, fetching the files is FX-3's step, and V1 downloads nothing here. The two checks it queues are unchanged.
3. **§Scope line**: "This covers the simulator half, plus what AD2 needs to know" understated the document once §7 and the §4 device-half bullet existed. Now: "This covers both halves V1 asks for — the simulator run and the acceptance-iPhone run of §7 — plus what AD2 needs to know."

## 7. Considered, not defects

- Line 103 "What `list_fonts` then shows is FX-3's call (open items below)" and §3's "a display question for FX-3": true as written — the question is FX-3's and the owner's answer is recorded there (the §4 bullet now says so); neither claims the item is still undecided.
- Verdict "Chinese needs a bundled CJK font in `Folders.fonts`; §4 proposes one": §4 does propose one, and the owner chose it; not a contradiction.
- `capture/review-devicectl.txt` still shows `unavailable`: the historical capture §1 cites in past tense; the current state is §1's row itself. The live `devicectl list devices` output was not written into `capture/` (canonical captures are canonical).
- §7's recipe keeps the `<device>` and `<team id>` placeholders; the concrete ids are in the round brief, and a reusable recipe is the point.
- `capture/b6-check/` needs no §8 row (§8's pattern row covers `<round>-check/` folders).

## 8. Outside the workspace

`~/dev/artcraft-dev`, `/tmp/craftbridge-v7` and `/tmp/craftbridge-amend` (the brief, read only) untouched. Scratch `b6-scratch/` (the section extractions, normalised files and diffs) was deleted after this file recorded their content; `capture/b6-check/` is kept as this round's device evidence. FontProbe was terminated at the end of its run and left installed; the booted simulator was not touched.

**Verdict: 3 defects fixed.**
