# Review round B8 — CraftBridge #28 (V1: list and read /System/Library/Fonts on iOS)

Round B8 re-checked round A7's four fixes by running them, then settled the open question A7 left in RESULT §4 — which routes can implement FX-3's `list_fonts` rule, and what the filtering route costs EffectCraft's drawing — by tracing every reader of the font registry at the pinned rev and measuring the cost on the simulator through EffectCraft's public API. It fixed what it found and promoted the captures whose content changed.

Work was alone. No font was downloaded, no GitHub request was made, no git state was changed and nothing was posted; cargo ran with `--offline` for everything new. EffectCraft was read only through `git show 103760584a60074c9aaeef686d2d05d89f08155e:<path>`; skrifa and read-fonts only through cargo's registry copies. The plan master was read read-only (§14, §15, §19, §21, §22, the PD4/PD5 rows, the acceptance table). The FX-3 decision was taken as given, because reading the issue needs GitHub.

## 1. Starting state and capture hygiene

- **Simulator:** iPhone 17 Pro, booted, iOS 26.5, left as found (FontProbe installed, from A7).
- **iPhone:** `xcrun devicectl list devices` now shows the acceptance iPhone `available (paired)` (A2–B3 had it `unavailable`). The app's sources had not changed since A7, so FontProbe was **not rebuilt or reinstalled**; it was only re-launched through `run-device.sh` with `CAPTURE_DIR=capture/b8-check`, per the round's rules, and nothing else on the phone was touched.
- **Canonical captures:** they had changed since the last copy in `capture/a7-run/` only in the sense that A7 itself left them; B8 copied nothing new to a `-run` folder except the one capture it replaced (A2's `review-devicectl.txt` into `capture/a2-run/`), because that is the only canonical capture whose content B8 changed. All of §5 (steps 1–5, plus the new 2b) ran with `CAPTURE_DIR=capture/b8-check`.

## 2. A7's fixes, re-run and re-read

| A7 fix | How B8 checked | Result |
|---|---|---|
| 1 (major): §4's `list_fonts` bullets — identity on both sides, result, where the filter sits, with citations | re-read A7's code changes line by line, then re-read §4 against the same code at the pinned rev: `tools.rs:222-224`/`:729-742` (`list_fonts` → `text.fonts`), `text_edit.rs:876-903` (the only command that emits `families` JSON), `fonts.rs:556` (`families()`, unfiltered), `fonts.rs:1169-1174` (`system_fallback` over registered faces), `file_more.rs:366-378` (`file.findMissing fonts` via `resolve`), `engine/fonts.rs:3-10`, `ui-egui/panels/essential.rs:437` | A7's citations are all correct and its *route 1* reasoning holds — **but its claim that keeping all 633 and filtering only the listing "needs a change to EffectCraft" is wrong for a CraftBridge app** (defect 1 below) |
| 2 (minor): the phone measures the face match itself (probe `face`/`uifamily` lines + `ecprobe_face_match`) | `run-sim.sh` and `engine-checks.sh` into `capture/b8-check/`; device re-launch | counts reproduce exactly: 266 matched / 86 families / 37 = 23 + 14 residuals, 0 unexplained, and `review-face-matching.txt` came back **byte-identical** to canonical. The device report's `face` and `uifamily` lines are unchanged from A7's (the only diffs are §8 instance values) |
| 3 (minor): one face-match implementation (`facematch.rs`), app and example share it, `coretext-faces.swift` deleted | file listing + re-run + clippy | the script is gone; the example's output byte-identical; `cargo clippy --all-targets` clean on the host, `aarch64-apple-ios-sim` and `aarch64-apple-ios` (re-run this round, including with the new example) |
| 4 (minor): the 14 residuals: names made up both ways, styles pair 8+3+3, coordinates not compared | §4 text vs the capture's sections 4–5 | holds; all 87 "file listed, face not" residuals sit in files whose base face matched, as §4 says |

Also re-counted: `nm` on the device binary exports the **7** `_ecprobe_` functions (A7 added `ecprobe_face_match`); §5 still said "the 6 functions" (defect 2).

## 3. The trace: every route to the family list, at the pinned rev

Readers of `families()` (directly or via the same registry):

- `text.fonts` (`crates/engine/src/cmd/text_edit.rs:876-903`) — the only command that emits a `families` JSON; `list_fonts` is a thin wrapper (`crates/automation/src/tools.rs:222-224`, catalogue entry `:729-742`).
- The engine's own font helpers (`crates/engine/src/fonts.rs:3-10`: `text_families`, `font_styles`, `font_menu`, …) — consumed by the desktop egui menus (`crates/ui-egui/src/panels/essential.rs:437`, `text_panels.rs:311/754`, `properties.rs:405`). `ui-egui` is not a crate a CraftBridge app links (plan §15's crate list), so those surfaces are not reachable from an app.
- The script crate exposes `executeCommand` and runs engine commands through `Session::execute` (`crates/script/src/lib.rs:17,26-27`), so a script inside the app can call `text.fonts` directly.
- `file.findMissing fonts` (`file_more.rs:366-378`) reads `effectcraft_text::resolve` per text layer — not a listing, but the same registry decides what counts as missing.
- Fallback (`fonts.rs:1100-1180`): usual families via `family_faces`, then `system_fallback` over every registered face whose origin is not `bundled`. `add_source` (`fonts.rs:398-410`) dedups case-insensitively by (family, style) and clears the *family-faces* cache only — the fallback search cache (`fonts.rs:1058,1128-1139`) survives it, which is why the 266-vs-633 comparison must run as two processes.

Inside a CraftBridge app all of it is reached through CraftBridge's own crate: `exec` and `mcp` are `craftbridge-effects` methods (plan §14 method table), the generated Swift/Kotlin facades route every typed call as `request("exec", {command, params})` (§21), and the plan puts all adaptations in `craftbridge-effects` on public interfaces (PD5, §15 adaptation table AD2 row, §19). So **CraftBridge can filter its own `list_fonts`/`text.fonts` result inside its own crate while keeping all 633 faces registered** — no EffectCraft change — with the honest caveat that the app's `mcp(line)` path and scripts calling `text.fonts` bypass that filter until each is wrapped in turn (also inside `craftbridge-effects`).

## 4. The drawing cost of route 1, measured

New: `ecprobe/examples/fallbackcost.rs` + `review-scripts/fallback-cost.sh` → new canonical `capture/review-fallback-cost.txt` (`f335c663…`, byte-identical across two runs). Method (all answers from EffectCraft's public API):

- Same 633 faces, dirs and CoreText face list as facematch; matched set = the 266 by (file, PostScript name), asserted via `add_source` counts (filtered run adds 266 → 273 registered; full adds 633 → 640).
- Sample per family of the 367 unregistered faces (225 families): the family's first character that a member's `cmap` maps to a glyph (skrifa `Charmap::mappings` — the first condition of EffectCraft's own `covers`, `fonts.rs:1017-1024`, on the same bytes) and that no kept face's `cmap` maps and no bundled `has_char` covers.
- Two clean simulator processes (`simctl spawn`, so `scan_system()` adds nothing): each registers its set via `fonts::add_source` and answers every sample with `fonts::fallback_for_locale(c, Inter Regular, "en-US")`; "none" = only a bundled face answers.

Result: **134 sampled families; under all 633 every one answers with a real face (nearly always the family itself); under the 266 every one answers none.** The other 91 families lose nothing (their whole `cmap` is already mapped by kept faces). The 134 are the script coverage of route 1: the 102 `UnicodeSupport` Noto families (Adlam, Cuneiform, Egyptian Hieroglyphs, Mongolian, …) plus internal faces (`.SF Arabic`, `.SF Devanagari`, `.CJK Symbols Fallback …`, `.PhoneFallback`, …). Under 633 the answers also show re-substitutions inside the 367 (Tagbanwa's sample → Noto Sans Buhid, Yi → Noto Sans PhagsPa), which is why every 633 answer is a real face.

## 5. Defects found in round B8, and fixes

1. **major — RESULT §4 told FX-3 the wrong thing about where the filter can sit.** A7's rewrite concluded: "Keeping all 633 registered for drawing and filtering only what is listed needs a change to `text.fonts` and the font menus, that is to EffectCraft." For a CraftBridge app that is false: `list_fonts`, the generated `text.fonts` call and `exec`/`mcp` all land in `craftbridge-effects`, and PD5/§15/§19 put exactly this kind of adaptation there (plan: "EffectCraft is never modified … adapted inside CraftBridge on EffectCraft's public interfaces"). The trace (§3 above) and the new capture made both routes and their real costs visible, so §4's bullet was rewritten as *two routes*: route 1 (filtered `FontSource`, registry invariant, costs the 134 script families' drawing — measured) and route 2 (keep 633, filter the listing in `craftbridge-effects`, costs nothing in drawing, the rule escapable via `mcp`/scripts until each path is wrapped, `count` re-computed, corpus-scoping caveat (§28)); neither changes EffectCraft, and the owner's choice between them remains recorded as FX-3's.
2. **minor — §5's step 3 still said "the 6 functions of ecprobe/ecprobe.h".** A7's `ecprobe_face_match` made it 7 (verified with `nm` on the device binary this round). §5 now says 7 and records the count.
3. **minor — RESULT §1 and `capture/review-devicectl.txt` described a phone state that had changed.** The phone was `available (paired)` again and the rounds' re-check sentence had gone stale. §1 and §7 now record B8's re-check and the re-launch of the unchanged installed app (every count reproduced, diffs = §8 instance values only); the capture was promoted, A2's state preserved in `capture/a2-run/review-devicectl.txt`.

Not counted as a defect: the device `stdout`/`stderr` capture split differs between runs (devicectl put its tunnel/launch banner on stdout this round, inside stderr in A7's run) — tool-stream timing chatter, no content effect; the report itself matched to instance values.

## 6. Files and captures this round

- New: `ecprobe/examples/fallbackcost.rs` (`03617090…`), `review-scripts/fallback-cost.sh` (`763e91eb…`), canonical `capture/review-fallback-cost.txt` (`f335c663…`), promoted `capture/review-devicectl.txt` (`3e2ddc6b…`).
- Changed: `ecprobe/Cargo.toml` (`35a6266f…`, adds `skrifa = "0.44"` — already locked transitively, no new package, built `--offline`), `ecprobe/Cargo.lock` (`fe874038…`, the dependency edge only). RESULT updated: scope line, §1 iPhone row, §4 (filter bullet → routes), §5 (step 2b, nm 7), §7 (B8 re-run), §8 (hashes, new rows, a2-run note).
- `capture/b8-check/` holds this round's check run (§5 steps 1–5 + 2b + device re-launch): all review captures byte-identical to canonical except the reports/stdout/stderr/oslog (instance values) and the promoted devicectl state.
- Side effects: the booted simulator ran FontProbe again (Documents report overwritten, as always); the phone ran the already-installed FontProbe via `devicectl` launch only; `build/fallback-cost-work/` and scratch were deleted; nothing outside `/tmp/craftbridge-v1` was written, and `~/dev/artcraft-dev/artcraft-plugin-port`, `/tmp/craftbridge-amend` and `/tmp/craftbridge-v7` were not touched.

**Verdict: 3 defects fixed (1 major, 2 minor); FX-3's `list_fonts` rule now has both routes traced and cited — neither needs an EffectCraft change — and route 1's drawing cost measured: 134 script families lose every covering face under the 266, 91 lose nothing.**
