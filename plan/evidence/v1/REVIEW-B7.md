# Review round B7 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B7 did three things. First it re-checked A6's fixes by running, not reading: the whole §5 check set went into `capture/b7-check/` and A6's five counts were re-derived there. Then it measured the owner's new FX-3 decision (craftbridge#113: `list_fonts` shows exactly the faces CoreText offers the app, matched face by face, plus the bundled fonts, so the families are `UIFont.familyNames`; the 93-family file rule is the fallback only if face matching cannot work) and rewrote RESULT §4 to the decision and the measurement. Finally the iPhone, available this round, was re-run.

Work was alone. No font was downloaded, no GitHub request was made, no git state was changed, nothing was posted, and the simulator was left as found. The FX-3 decision was taken as given from the round brief; reading the issue itself needs GitHub, which this round does not use. EffectCraft was read only through `git show 10376058…:crates/text/src/fonts.rs` and `…:crates/text/src/sfnt.rs`.

## 1. Environment and starting state

- **Simulator:** iPhone 17 Pro, booted, iOS 26.5, as in every round before it. FontProbe was reinstalled by `run-sim.sh`; the other app there was not touched.
- **Device:** `xcrun devicectl list devices` shows "SeaEl iPhone" **`available (paired)`** this round, so it was re-run (§4). The canonical `review-devicectl.txt` still shows `unavailable`, as it did when it was captured; §1's row explains both states and B6 set the precedent of not promoting the transient connection state.
- The canonical captures were hashed before anything ran: every `capture/review-*`, `sandbox-canary.txt` and `capture/device-*` matched the §8 record (report `f2844cb0…`), so nothing needed copying to `capture/a6-run/`.

## 2. A6's fixes, checked by running

A6's capture `review-coretext-file-families.txt` came back **byte-identical** from this round's run, and with it every count A6 recorded: EffectCraft reads 123 families from the 103 files CoreText lists; 30 are dot-named; the 7 visible extras are exactly the six Hiragino Pro/Std cuts and Noto Sans Zawgyi, all among section 9's 141; with dot-named families hidden the file filter gives 93 = UIKit's 86 + those 7; and the report's §9 line still reads 81/86 by English name with the 5 Arabic-named families present under their Arabic names. A6's other two fixes (the §4 wording, the `-allowProvisioningUpdates` line instead of the uncaptured `No Accounts` claim) are in place.

## 3. The face-by-face rule, measured

The question was whether face matching can work at all, and what it gives. Both sides were taken by the same calls the app uses. The CoreText side is `review-scripts/coretext-faces.swift`, built for the simulator and run with `simctl spawn booted`: the same `CTFontCollectionCreateFromAvailableFonts(nil)` as the app's section 8, printing per descriptor its PostScript name (`kCTFontNameAttribute`) and file (`kCTFontURLAttribute`), then `UIFont.familyNames`. The EffectCraft side is `ecprobe/examples/facematch.rs`: `DirectorySource` over `review-coretext-dirs.txt`, keeping each face whose (file, PostScript name) pair — `FaceData::File` + `FaceInfo.postscript`, public API of the pinned rev — is one CoreText lists. `engine-checks.sh` now runs both and writes the answer into `capture/review-face-matching.txt`. The answer:

- CoreText: 303 descriptors, 103 files, 86 families as CoreText names them. EffectCraft: 633 faces from 263 files. Every one of the 303 and of the 633 carries a PostScript name, and no (file, PostScript name) pair repeats on either side — so the alternative identities the brief allowed for (family + style, the name table) were not needed anywhere.
- 266 EffectCraft faces match. **Their families are exactly `UIFont.familyNames`: 86.** The set difference is 5 + 5 and is fully the 5 Arabic renames (Al Nile → النيل, Damascus → ڈیمسکس, Farah → فرح, Geeza Pro → گیزا پرو, Mishafi → مِصحفي); unexplained names: 0.
- The 37 CoreText faces no EffectCraft face answers with the same pair are all descriptors of named instances of variable fonts. 23 are instances recorded on that file's matched face (`FaceInfo.instances[].postscript`, public): PingFang HK/MO/SC/TC's five non-Medium weights each, Kefa III's Light, Bold and ExtraBold. The other 14 — Noto Sans Syriac's eight weights and STIX Two Text's Bold/Medium/SemiBold plus the same three italic — name the same design-space instances differently (CoreText `NotoSansSyriac-Regular_Black`, `STIXTwoText_Bold`; EffectCraft `NotoSansSyriac-Black`, `STIXTwoTextRoman-Bold`); the styles pair off one to one and both families are on the 86 through their base faces. Both sides' names stand in the capture.
- The rule needs no new EffectCraft API — `FontSource::faces()`, `FaceInfo.postscript`, `FaceData::File`, plus `FaceInfo.instances` to account for the instances — and it does not change the registration or the fallback: only the listing filters.
- Measured in the simulator. Sections 8 to 10 of the device report matching the simulator's, file sizes included (§7), means the phone holds the same 103 files, so the same 86 come out there; the phone was not re-run for this alone.

## 4. The check runs (`capture/b7-check/`)

`run-sim.sh` and `engine-checks.sh` ran with `CAPTURE_DIR=capture/b7-check`, plus the §5 step-4 typecheck (exit 1, the one expected error, capture byte-identical to canonical) and step-5 devicectl; step 3 rebuilt the iphoneos target unsigned: BUILD SUCCEEDED, `nm` lists the same six `_ecprobe_` functions. Step 6 was not run: it needs GitHub. Compared with `capture/`: **11 review files byte-identical** (the folder lists, family lists and diff, file-families, Arabic names, Han fallback, outline formats, host-checks, typecheck); the report and stdout, stderr and os_log differ only in instance values (40 lines: timestamps, pids, XPC id, `TERM`, ms timings, the body size and SHA they move); `review-devicectl.txt` shows the device available; `review-face-matching.txt` is new content and was promoted to `capture/`.

The device was re-run with `run-device.sh 08452DF6-…` (sources unchanged since the canonical run, so the installed app was reused; the script only launches and copies). Its report differs from the canonical device report in instance values only — timestamp, pid, `TERM`, XPC id, the ms timings, and the `st_dev` volume number of section 1, which moved 822083585 → 838860801 with a remount; every count is the same and `READ-BACK` is OK. With timings normalised, its sections 8 to 10 are byte-identical to the canonical device report's, and against the canonical simulator report they differ exactly in the ways §7 documents. B6's and A6's §7 and §8 claims all hold again.

## 5. Defects found in round B7, and fixes

1. **major — RESULT §4 recorded `list_fonts` as an open filter question, which the owner has since decided on FX-3, and nothing in RESULT showed the decision can be done.** A6's bullet (and the two sentences pointing at it) said the decision "leaves the filter to FX-3, and neither simple filter gives UIKit's 86". The owner's decision is face-by-face matching plus bundled fonts, with the file rule only as fallback. **Fix:** §4 now carries the decision and the measurement of §3: the new decision bullet (identity used, 266 faces → exactly the 86 families, 0 missing and 0 duplicated PostScript names, no new API needed, same result on the phone and why), the residuals bullet (the 37, all instances, 23 found via `FaceInfo.instances`, 14 named differently, no family lost), and the old 81/123/93 numbers kept as the fallback's record. The pointer in the first sub-bullet, the Chinese bullet's "only the families CoreText lists" and §5's step-2 comment were synced to the decision. `engine-checks.sh` writes the new capture; `coretext-faces.swift` and `facematch.rs` are new sources; §7 gained the sentence that the identical sections 8–10 make the simulator measurement carry to the phone; §8 gained the capture row and the three source rows.
2. **minor — §8's method paragraph missed two things A6 §5 had flagged or this round exposed.** Its copy rule named only `capture/review-*` and the canary, not the canonical `capture/device-*`; and its instance-values list did not cover the `st_dev` volume number, which moved in this round's device re-run. **Fix:** both added, with the remount explanation.
3. **minor — the scope line still stopped the rounds list at A5** although B6 and A6 also corrected RESULT. **Fix:** extended through B6, A6 and B7.

## 6. Considered, not defects

- **The devicectl capture.** Canonical shows `unavailable`; this round's shows `available (paired)`. That is the cable, not a finding; §1's row already states both states, and B6 left it the same way.
- **The 266-of-633 gap.** 367 EffectCraft faces match nothing: 87 sit in files CoreText lists but carry names CoreText does not (the dot-named `.PingFang UI …`, `.… Interface`, `.… PUA` cuts, the 7 visible extras), 280 are in the 160 files CoreText does not list — already §3's story of the walk versus the collection, and §5 of the capture attributes every one of them.
- **The 14 differently-named instances:** their coordinates were not compared, only their styles; styles pair off 1:1 and the families match through the base faces, which is what the decision asks for.
- **Kefa III, PingFang and the rest of the variable fonts** each matched through one base face; that a family can be represented by one face and still list correctly is exactly what a family-level `list_fonts` needs.
- **The sim's own `st_dev` values** (16777232 / 16777239, §2) are unchanged in this round's report, so §2 still quotes live numbers.

## 7. Agreement with the recommendation

**Agree.** The AD2 recommendation is unchanged and was re-verified end to end. For FX-3, §4 now states the implementable rule: keep a family's face when its (file, PostScript name) pair is one CoreText lists, add the bundled families, and the check is `UIFont.familyNames` — 86, five of them under EffectCraft's Arabic names. Nothing in the measurement pushes FX-3 toward the file-rule fallback.

## 8. Open items

- **FX-3 implements the filter**; `list_fonts` is a listing view — the registration and fallback keep all 633 faces (drawing needs the extra script fonts).
- **The bundled fonts:** family names inside the two Noto files, and the fallback with them, stay unverified until FX-3 fetches them (§4).
- **The device is connected and paired again**; later rounds can re-run it if §7's claims need re-checking.

## 9. Outside the workspace

- Read-only: `simctl` and `devicectl` queries, the SDKs, and the two EffectCraft files read through `git show` at the pinned rev.
- The simulator's `data/tmp/fontprobe-stdout.txt` and `fontprobe-stderr.txt` were overwritten by `run-sim.sh`, as in earlier rounds. `build/coretext-faces` is this round's new build product, rebuilt by `engine-checks.sh` whenever it runs.
- Scratch (`build/face-match-test/`, `build/face-match-work/`, `build/b7-scratch/`, a copy of the pinned `fonts.rs`, a baseline-hashes file in `/tmp`) was deleted at the end. `capture/b7-check/` is kept as this round's check run.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` (beyond the round brief) and `/tmp/craftbridge-v7` were not touched.

**Verdict: 3 defects fixed (1 major, 2 minor); the face-by-face `list_fonts` rule measured — 266 faces, exactly UIKit's 86 families by (file, PostScript name) — and A6's work reproduced; recommendation agreed.**
