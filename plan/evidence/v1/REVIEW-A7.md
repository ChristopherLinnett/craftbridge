# Review round A7 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round A7 re-checked round B7's face-matching work by running it, then reviewed the whole investigation again. It read both new sources critically and checked RESULT §4 against EffectCraft's own code at the pinned rev. It then fixed what it found, measured the `list_fonts` rule on the iPhone, and promoted the captures whose content changed.

Work was alone. No font was downloaded, no GitHub request was made, no git state was changed and nothing was posted. EffectCraft was read only through `git show 103760584a60074c9aaeef686d2d05d89f08155e:<path>`, and skrifa and read-fonts only through cargo's registry copies. The FX-3 decision was taken as given, because reading the issue needs GitHub.

## 1. Environment and starting state

- **Simulator:** iPhone 17 Pro, booted, iOS 26.5. The third-party apps before and after the round were FontProbe (reinstalled by `run-sim.sh`) and `com.example.orchestratorFlutter` (not touched).
- **iPhone:** `xcrun devicectl list devices` showed the acceptance iPhone `available (paired)`. The app's source changed this round (§4), so FontProbe was rebuilt, signed with team `<team id>` as in §7's recipe, and installed with `devicectl device install app`. Only FontProbe was run, through `run-device.sh` with `CAPTURE_DIR=capture/a7-check`. Nothing else was installed, no setting was changed, and FontProbe stays installed.
- **Canonical captures:** they had changed since the last copy in `capture/a4-run/`: A6's and B7's new captures and the device captures. So round A7 first copied all 23 (`capture/review-*`, `sandbox-canary.txt`, `capture/device-*`) to `capture/b7-run/`, and hashed them.

## 2. B7's claims, re-run

| B7 claim | How A7 checked | Result |
|---|---|---|
| `engine-checks.sh` makes `capture/review-face-matching.txt`, and the canonical capture holds 266 / 86 / 37 / 23 / 14 | `run-sim.sh` and `engine-checks.sh` into `capture/a7-check/` with B7's code, untouched | the capture came back **byte-identical**, along with every other review capture, so the counts hold: 303 descriptors, 103 files, 86 families; 633 faces from 263 files; 266 matched; 23 instances plus 14 others = 37; 86 families; 5 renames; 0 unexplained |
| no PostScript name missing or duplicated on either side | the capture's section 3, and `sfnt.rs:122` at the pinned rev | 0 / 0 / 0 / 0. EffectCraft's `postscript` is the face's own name ID 6 and is never synthesised, so "0 missing" means something |
| the 23 are named instances found via `FaceInfo.instances[].postscript` | the capture's residual lines | 20 PingFang (HK/MO/SC/TC × Light, Regular, Semibold, Thin, Ultralight) and Kefa III's Light, Bold and ExtraBold, each tied to the file's matched variable face |
| the 14 "pair off one to one" | compared CoreText's 14 against the instance names EffectCraft lists for the same files | they do, by style. Noto Sans Syriac's 8 match EffectCraft's 9 instances less Regular; STIX Two Text's roman 3 match its 4 less Regular; its italic 3 match its 4 less Italic. Neither side's names come from the font: CoreText appends `_Style`, EffectCraft derives `<prefix>-<Style>` (`sfnt.rs:135`, `:191`). Coordinates were not compared, which B7's review notes but RESULT did not (defect 4) |
| file paths compare the same way on both sides | traced both sides | in the same strings. The folders in `review-coretext-dirs.txt` are the parents of the app's CoreText URL paths (`URL.path`, percent-decoded: the `iOS 26.5.simruntime` space matched). `walk` joins entries onto those strings and keeps no canonicalised path, and there are no symlinks under the runtime fonts (host-checks). Every one of the 103 files has a matched face: all 37 residuals sit in files whose base face matched. So a path-form mismatch cannot hide anywhere |
| can a match succeed for the wrong reason? | checked the identity and the registration | no. Pairs are unique on both sides, and EffectCraft's names are the font's own. `add_source`'s family-and-style dedup changes nothing, because all 633 faces were added (section 9: "add_source added 633") |
| the device re-run matched apart from instance values | B7's capture vs the canonical device report | not re-checked separately; this round's own device run (§4) supersedes it |
| "no new EffectCraft API is needed … only the listing filters" | read EffectCraft's `list_fonts` chain at the pinned rev | **does not hold** (defect 1) |
| "the same 86 come out there" (the phone) | RESULT §7's basis | inferred, not measured, and "same sizes" covers only section 10's six files and the rounded MiB total (defect 2) |

## 3. Defects found in round A7, and fixes

1. **major — RESULT §4's `list_fonts` rule could not be implemented as written.**
   - **What RESULT and REVIEW-B7 said:** the match "needs no new EffectCraft API" and leaves "the registration and the fallback" alone, "only the listing filters".
   - **What the pinned rev does:** EffectCraft's MCP `list_fonts` runs `text.fonts` (`crates/automation/src/tools.rs:222-224`). That lists `effectcraft_text::families()` (`crates/engine/src/cmd/text_edit.rs:876-903`), which lists every registered face's family with no filter (`crates/text/src/fonts.rs:552-576`). The engine's font lists and an editor font menu read `families()` too (`crates/engine/src/fonts.rs:3-10`, `crates/ui-egui/src/panels/essential.rs:437`).
   - **The consequence:** with EffectCraft unchanged, the rule can only be done by registering only the matched faces: a `FontSource` wrapping `DirectorySource` (public API). That narrows the fallback, which searches only registered faces (`fonts.rs:1169-1174`), to those 266 faces plus the bundled ones. Keeping all 633 faces for drawing and filtering only what is listed needs a change to EffectCraft. Nothing in RESULT said where the filter goes, and FX-3 would have found out by trying.

   **Fix:** §4's decision bullet now names the identity on both sides with the exact attributes and fields, the result, where the filter has to sit (both routes and what each costs, with the citations above), and the check. It states that the decision as recorded does not say which route it means, so FX-3 has to settle that. The first AD2 sub-bullet now points at the tension.
2. **minor — "the same 86 on the phone" went beyond the evidence.** §7 said the phone and the runtime "hold the same 103 font files: same names, same sizes". Sizes were compared for six files only, and the phone's PostScript names were never captured. **Fix:** the face match is now measured on the phone.
   - **The probe change:** section 8 prints every descriptor's identity as a `face` line (PostScript name, file, family) and every `UIFont.familyNames` family as a `uifamily` line. Section 9 runs the face match in-app through a new `ecprobe_face_match`.
   - **The device result:** 303 descriptors, 103 files, 86 families; 633 faces; 266 matched; 23 + 14 residuals; 86 families; 5 renames; 0 unexplained.
   - **The comparison with the simulator:** its 303 `face` lines and 86 `uifamily` lines equal the simulator's line for line once the runtime-root prefix is removed.

   §4, §7 (new table row, rewritten sentence) and the Verdict bullet now say "measured on the phone".
3. **minor — duplicated code.** B7's `review-scripts/coretext-faces.swift` was a second program repeating the app's section-8 CoreText descriptor loop. **Fix:**
   - the app now prints those identities itself, so the loop exists once;
   - `engine-checks.sh` reads them from the report's `face` and `uifamily` lines, and the script is deleted;
   - the face-match computation moved from the example into one library module, `ecprobe/src/facematch.rs`, used by the `facematch` example (full detail) and the app (summary, through `ecprobe_face_match`).

   The example's output is byte-identical to B7's for the same input. The capture's sections 1–2 come out with the same content from the report as from B7's program, so the app process and the spawned CLI see the same 303 identities; only the provenance header lines changed. `cargo clippy --all-targets` is clean for the host, `aarch64-apple-ios-sim` and `aarch64-apple-ios`.
4. **minor — RESULT §4 said the 14 "name the same design-space instances differently".** Only their style names were compared, never their coordinates. **Fix:** it now says how each side makes the names up (CoreText's `_Style` suffix; EffectCraft's ATN #5902 derivation, `sfnt.rs:135`, `:191`), that the styles pair off 8 + 3 + 3, and that the coordinates were not compared.

Also tidied, not counted: in the face-match code, a meaningless `{:+}` flag on a string and an always-true condition after an earlier `continue`. Output is unchanged.

## 4. The check runs (`capture/a7-check/`) and what was promoted

- **Run 1, B7's code:** every review capture was byte-identical to canonical, apart from the report, stdout, stderr and os_log (instance values only).
- **Run 2, after the fixes** (simulator; the probe was rebuilt, not a clean build): every capture whose content does not depend on the new lines is byte-identical to canonical.
  - The report gains 303 `face` lines, 86 `uifamily` lines and the face-match summary, and is otherwise instance values only (88,087 B).
  - `review-face-matching.txt` differs only in its two provenance header lines.
  - Typecheck: exit 1, byte-identical.
- **iPhone run** (rebuilt app): exit 0; stdout body == Documents copy; 72,056 B. Against the canonical device report it differs only in instance values (pid, timestamps, container paths, XPC id, `st_dev` 822083585 → 838860801, ms timings, body size and SHA) and the new lines. Normalised against the simulator report, sections 8–10 differ exactly as §7 says: the runtime-root count line, the runtime-copy scan, and the `/System/Library/Fonts` counts.
- **Promoted to `capture/`,** because their content changed:
  - `review-documents-report.txt`, `review-stdout.txt`, `review-stderr.txt`, `review-oslog.txt`;
  - `review-face-matching.txt`;
  - `device-documents-report.txt`, `device-stdout.txt`, `device-stderr.txt`, `device-copy.txt`.

  RESULT's run record (scope line, §3's "23 ms in the canonical report's run", §8's report and device rows) now names round A7's runs, and §8's hashes follow every changed file. B7's set is in `capture/b7-run/`.

## 5. Agreement with the recommendation

- **AD2:** agree, unchanged. Register the parent folders of CoreText's files, now confirmed on the phone with the face identities themselves.
- **FX-3:** RESULT now gives an implementable rule: the identity, the public API (a filtering `FontSource`) and the check. It also records honestly that the rule, done with EffectCraft unchanged, gives up fallback coverage from the 367 faces CoreText does not offer.
- **The file rule:** it stays recorded as the fallback, with its 81 / 123 / 93 numbers.

## 6. Open items

- **FX-3:** choose between registering only the matched faces, which narrows the fallback, and changing EffectCraft's `text.fonts` and its font menus to filter only the listing. The decision as recorded in RESULT does not say which.
- **The bundled Noto fonts:** their family names, and the fallback with them, stay unverified until FX-3 fetches the files.

## 7. Outside the workspace

- `xcodebuild -allowProvisioningUpdates` talked to Apple's developer service to sign the device build (the §7 recipe). `devicectl` installed and ran FontProbe on the iPhone.
- The simulator's `data/tmp/fontprobe-*` files were overwritten by `run-sim.sh`, as in earlier rounds.
- Scratch `a7-scratch/` (hashes, logs, edit scripts, the regenerated example inputs) was deleted at the end.
- `capture/b7-run/` and `capture/a7-check/` are kept, as §8's pattern describes.
- `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 4 defects fixed (1 major, 3 minor); the face-by-face `list_fonts` rule re-derived (266 / 86 / 37 = 23 + 14) and now measured on the iPhone too; recommendation agreed.**
