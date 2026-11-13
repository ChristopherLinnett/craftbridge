# Review round B4 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: build side, round B4, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md` and
`REVIEW-A4.md` as handed over after round A4, `REVIEW-B3.md`, the earlier review files, `app/`, every
APK set, `evidence/`, `docs/`, `logs/`, the scripts. Method as before: new evidence only under
`CASE_TAG=review-b4` (APK directory passed to `queue.sh` explicitly), earlier evidence and review
files untouched, a re-run that agrees with earlier evidence apart from instance values is not a
defect and does not change RESULT.md.

**Verdict: 2 defects found and fixed (D58, D59), both left over in the round bookkeeping A4 rewrote: an artifacts line that mis-states which AOSP sources are saved, and a parenthetical claiming `apk_md5=` is in every `case.txt` when the 49 build-side cases predate that field. A4's own fixes (D54–D57) and all of its re-runs re-verified clean. The recommendation, its three amendments and every measured result stand. Not clean.**

## 1. A4 claims and fixes checked

| A4 claim / fix | How checked | Result |
|---|---|---|
| D54: every gap in the android15 `onFgsTimeout` block is now marked, and the android14 block is still verbatim | own extraction + gap-check script: strip indentation, locate each non-`…` line of each fenced block in order in its `docs/` copy, then demand an `…` line wherever the source advances by more than one line | true: block 1 = contiguous `aosp-android14-release-ActiveServices.java` lines 2311–2318 (no internal gap, none needed); block 2 maps to android15 lines 3900 / 3905–3906 / 3909 and both gaps are marked — the first hides exactly the four lines A4 described (3901 `// Discard any other messages for this service`, 3902–3904 the two `removeMessages` calls), the second the two `obtainMessage` lines 3907–3908 that were already marked |
| D55: both fixed quotes now match their pages character-for-character | own quotation script (`logs/b4-quotes-pass2.txt`): every double-quoted passage in RESULT.md outside fenced blocks, per paragraph (63 distinct after normalising markup/whitespace/apostrophes, split at `…`), each segment searched in every `docs/*.txt`/`docs/*.html` | true: `will also be called when a foreground service of type ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE times out` → `docs/service-ref.txt`; `has time limit of 6 hours starting from Android version Build.VERSION_CODES.VANILLA_ICE_CREAM` → `docs/serviceinfo-ref.txt`; every other page-attributed quote found on its page (timeout page, service-types, awake, play-console, datasync-migration, Service/ServiceInfo refs). The only non-matches are not page quotes: five scare quotes/labels (`mediaProcessing\|dataSync`, "extra second", "no type on old devices", "autostart", "Swipe to dismiss from recents" — none presented as a quotation in context) and three source comments, each found verbatim in the saved java instead (`Unknown type, fallback to the defaultToType` Policy:1350; `…currently in the TOP state or was in the TOP state after the time limit was exhausted previously` ActiveServices:2447–2448; `Log a WTF instead of crashing the app while the new behavior is gated.` ActiveServices:3976) ✓ |
| D56: RESULT.md now describes both branches of the flag correctly, at the cited lines | `grep -n enableFgsTimeoutCrashBehavior docs/aosp-android15-release-ActiveServices.java` then read both regions in full | true: the flag is used exactly twice in the saved file, 2459 and 3964 — RESULT's "lines 2459 and 3964" is exact. At 2451–2465 the exhaustion branch throws `ForegroundServiceStartNotAllowedException(exceptionMsg)` only when the flag is on, else `Slog.wtf(TAG, exceptionMsg); fgsTypeInfo.reset();` — matching RESULT's "throws … only when … is on; with the flag off the platform logs the message as a WTF and resets the record anyway"; the reset-condition quote and "(or the first start of the type was more than 24 h ago)" match the source comment/conditions at 2441–2448; at 3964 `onFgsCrashTimeout` crashes only with the flag on, else the quoted comment + `Slog.wtf` ✓. Nothing else in the file gates on the flag, so "the same flag decides" is complete |
| D56: the flag's state on the API 37 image is not established (A4's finding + "RESULT.md's new note states it accurately") | re-ran every A4 check (`logs/a4-flag37.txt` line by line) and added every further non-root avenue I could think of, transcribed to `logs/b4-flag37.txt`; tool existence settled by running it | confirmed, and unchanged: `aflags` exists on the image (its help lists `list`/`enable`/`disable`/`unset`; there is no `get` subcommand; `aflags list` → `Error: must be root`); the flag name is absent from every flag store — all four `aconfig_flags.pb` (system/system_ext/product/vendor), `/system/etc/aconfig/flag.info`, `flag.map`, `flag.val`, `package.map` (control `fgs_delegate_system_api` present in each store that holds it) — A4's `/apex/*/etc/aconfig_flags.pb` glob really matches no file (`find /apex -maxdepth 3 -name aconfig_flags.pb` is empty). New avenues, all negative: `getprop` has no matching property, `device_config get android.app enable_fgs_timeout_crash_behavior` → `null`, legacy `settings get global …` → `null`, `dumpsys device_config` (readable without root) lists other fgs keys but not this flag. A string search of the image's own binaries finds the flag name in none of `services.jar/.odex/.vdex` or `framework.jar`, while the same binaries do contain the runtime messages "Time limit already exhausted" and "did not stop within its timeout" — consistent with the flag having been rolled forward into a build-time constant on that branch, whose value no non-root surface can show. RESULT.md's sentence ("… which way a given Android 15–17 build behaves was not established here") stays accurate; advice unchanged |
| D57: round bookkeeping rewritten round-independently | read the header, end-state pointer, per-round build list, artifacts and label lists; then checked them against today's state: header pointer "the newest file is the latest round" ✓ (mtime), §7 end-state ✓; recomputed `apk_md5=` of all 209 `case.txt` against the per-round sets (comparing per flavor, since not every case runs the combo: `review-a2-*` `8e76be90` = apk-b1 **full**, `review-b1-*` `15b2745f`/`8e76be90` = apk-b1 **noperm**/**full**, the `review-a1` oddities `c84df81a`/`2d6d8586` = apk2 full/none — all in `evidence/apk-md5.txt`; zero real violations); "This report cites no `review-b2-*` case" ✓ (grep: the only `review-b2` hits are the claim itself); Environment field-notes ✓ (all 49 build-side `case.txt` lack `device_sdk=`, review-a1+ have it; `target_sdk=` missing exactly from the pre-fix `review-b1-*` harness cases A4 says ran the older harness; `home_sent_device=` missing exactly from cases that died at launch before HOME was sent, which is what the harness does) | mostly true — the rewrite itself holds and cannot go stale on this round's additions (header, end-state, build list, label list all survive round B4 unchanged) — but two lines A4's rewrite touched are wrong as written: the `docs/` artifacts line and the `(apk_md5= in every case.txt)` parenthetical → **D58, D59** (below) |
| A4 §2: all 7 re-runs agree with B3 | re-ran the required subset myself (§2): a29 amendments queue + a37 crash; plus A4's own artifacts: `review-a4-a29-combo-auto-long` `typeBit=1 → EXPORT_DONE elapsedS=660 fileKiB=420864`, `dumpsys_bg` `isForeground=true`, `resumed_bg` launcher resumed, `dumpsys_late` `(nothing)`; A4 crash deltas recomputed from its own logs: +41 ms (29), +77 ms (37); `review-a4-a29-combo-mediaproc` has a foreign `FATAL EXCEPTION` in its log and no crash field (D52 holds) | agree on every point; my re-runs (§2) match A4's apart from instance values |
| A4 §3 line-number and source claims | re-grepped each: `onFgsTimeout` at 3887 and `onFgsCrashTimeout` at 3947 (android15 `ActiveServices`); "Only throw an exception if the new crashing logic gate" comment at 2457; `DefaultForegroundServiceTypePolicy()` at 1305 with `will default to "none"` at 1335 and the fallback method at 1347–1356; `setServiceForegroundInnerLocked` starts 1993, `validateForegroundServiceType` at 2552 with `FGS_TYPE_POLICY_CHECK_DISABLED` → `InvalidForegroundServiceTypeException` at 2573/2581 | all true ✓ |

## 2. B4 re-runs (`CASE_TAG=review-b4`, `apk-b1/`, new folders under `evidence/`)

Queue: `CASE_TAG=review-b4 queue.sh emulator-5554 29 amendments /tmp/craftbridge-v7/apk-b1`
(→ `logs/b4-queue29.log`) plus one direct `run_case.sh` on 37. `device_sdk=` matched every label.

| case | result | vs earlier evidence |
|---|---|---|
| `review-b4-a29-combo-none` | `EXPORT_DONE`, `STARTFOREGROUND_OK requested=none typeBit=0` | agrees with A4/B1 |
| `review-b4-a29-combo-short-notdeclared` | `PROCESS_GONE_AT_LAUNCH`, `STARTFOREGROUND_FAIL … 0x00000800 is not a subset … 0x00002001`, `fatal_exception_at_launch=android.app.RemoteServiceException`, FATAL +49 ms after the caught exception | agrees (A4 +41 ms, A3 +84 ms — instance spread under "dies within a second") |
| `review-b4-a29-combo-mediaproc` | `STARTFOREGROUND_OK requested=mediaProcessing typeBit=8192 → EXPORT_DONE`; its log holds the previous case's foreign `FATAL EXCEPTION`; no crash field | agrees (D52 extraction intact) |
| `review-b4-a29-combo-datasync` | `typeBit=1 → EXPORT_DONE` | agrees |
| `review-b4-a29-combo-bad-type-name` | `APP_CRASHED`, `fatal_exception=java.lang.RuntimeException;java.lang.IllegalArgumentException` | agrees |
| `review-b4-a29-combo-auto-long` | `STARTFOREGROUND_OK requested=auto typeBit=1 → EXPORT_DONE type=auto elapsedS=660 fileKiB=417344`; background `dumpsys_bg` `isForeground=true`, `resumed_bg` = launcher, `dumpsys_late` `(nothing)` | agrees (earlier runs 413,632–421,504 KiB; A4 420,864, B3 417,280) |
| `review-b4-a37-combo-short-notdeclared` | `PROCESS_GONE_AT_LAUNCH`, `fatal_exception_at_launch=android.app.RemoteServiceException$ForegroundServiceDidNotStartInTimeException;android.app.StackTrace`, FATAL +43 ms after `STARTFOREGROUND_FAIL` (identical not-a-subset message) | agrees (A4 +77 ms; same class) |

No re-run contradicted earlier evidence, so per the method no runtime claim changed; the two RESULT.md
edits are the bookkeeping fixes below.

## 3. Artifacts and environment claims checked against the tree

- `evidence/`: 209 `case.txt` (+ my 7 new = all present); `evidence/apk-md5.txt` covers all 33 APK
  files; `review-a1-build-compare` exists as listed.
- `logs/`: the line "b1-* = review B1 runs and cross-checks; the b1-run*.sh / b1-reset37.sh files …"
  matches the directory (all listed files exist); `a4-pids.txt`, `a4-final-state.txt`, `a4-flag37.txt`
  as cited; the "a2-\*, b2-\*, a3-, … = later rounds" and "`<round>-pids.txt`" wording also covers my
  `b4-*` files without going stale.
- `docs/`: every cited developer page has its saved copy (verified via each saved HTML's canonical
  link, except the support.google.com Play page, which carries none) and the two declared live-only
  pages are indeed absent — but the AOSP enumeration in the same
  line does not match the directory, and three captures are unaccounted for → **D58** (fixed). The
  saved `datasync-migration.html` is the current page served at the cited URL (its canonical is now
  `background-tasks/data-transfer-options`; every quote still matches the saved copy).
- Two source-level figures re-verified independently: android15 `FGS_INTRODUCE_TIME_LIMITS =
  317799821L` (ActiveServices:519) equals the change id the device's own `dumpsys platform_compat`
  recorded in the `timeout_knob.txt` captures I checked (`ChangeId(317799821; …)` — one in each of
  the a1/a2/b1 API 37 knob captures); `FGS_TYPE_NONE_DISABLED_CHANGE_ID = 255038118L` (Policy:123).
  RESULT.md cites both correctly.

## 4. Defects found and fixed in RESULT.md

**D58 — the `docs/` artifacts line mis-states the saved AOSP sources and omits three stray captures.**
It claimed `ActiveServices`, `ActivityManagerConstants`, `ServiceRecord` and `ForegroundServiceTypePolicy`
were each saved "for android10/14/15-release". In fact only `ActiveServices` exists for all three
releases; `ActivityManagerConstants` is saved for 14/15 only, `ForegroundServiceTypePolicy` for 14
only and `ServiceRecord` for 15 only. The line also presented the web captures as exactly "the pages
cited", while `docs/` additionally holds `docs_tmp.html` (a second, HTML copy of the ServiceInfo
reference), `fgs-guide.txt` (the foreground-services overview page, never cited) and
`v12-changes.html`/`.txt` (a failed early fetch that saved a 404 page). Fixed: the line now lists
each class with the releases actually saved and names the three stray captures. An enumeration that
"lists all saved copies" and is wrong for half its entries is the same citation-fidelity failure D57
was; the round-independent wording around it is otherwise sound (round B4's new files fall inside
existing round-independent phrasing, so nothing else went stale).

**D59 — "`apk_md5=` in every `case.txt`" is false for the build-side cases.** The claim's
parenthetical reads "(`apk_md5=` in every `case.txt`)", but the 49 build-side (`a<api>-*`) case
directories predate the field and carry no `apk_md5=` line (confirmed: 209 `case.txt`, 49 without it
— exactly the build-side set). The surrounding build-list claim itself still holds (checked per
flavor, §1). Fixed: the parenthetical now says the field is in every review round's `case.txt` and
notes that the build-side cases predate it and name their APK set in `case.txt`'s `apk=` path.

Files changed this round: **`RESULT.md`** (the two fixes above, both in the Environment/Artifacts
blocks; no runtime claim, table or verdict touched) and **`REVIEW-B4.md`** (this file). New evidence:
`evidence/review-b4-*` (7 folders), `logs/b4-*` (`b4-pids.txt`, `b4-queue29.log`,
`b4-emulator-29.log`, `b4-emulator-37.log`, `b4-flag37.txt`, `b4-quotes-pass2.txt`,
`b4-final-state.txt`). All earlier rounds' evidence, logs and review files are untouched.

## 5. Agreement with the recommendation

The recommendation is unchanged by this round: declare `mediaProcessing|dataSync`, pass the type
explicitly (`mediaProcessing` when `Build.VERSION.SDK_INT >= 35`, else `dataSync`), never start
type-less, never rely on an untyped FGS for a target-34+ app, stop within seconds of `onTimeout`, and
start from the foreground while burning a type budget. Every point rested on evidence I re-ran or
re-read this round agrees with what is in RESULT.md.

## 6. Open items

- **`enableFgsTimeoutCrashBehavior` state on Android 15–17 builds** — still open, and now documented
  as exhausted at the non-root level: A4's avenues (flag absent from the image stores; `aflags` needs
  root — settled by running it, and settled against the tool's own help on the device) plus B4's
  additions (per-namespace `flag.info`/`flag.map`/`flag.val`/`package.map`, `find /apex`, `getprop`,
  `device_config get`, legacy `settings`, `dumpsys device_config`, binary string search of the image's
  own `services.*` and `framework.jar`) all come up empty; no documented non-root reader exists, and
  the flag name is not present in the image's framework binaries at all, so its build-time value on
  that image cannot be established from the device alone — it would need the image's build sources
  (not published for API 37) or root plus a rolled-back flag store. Advice already covers both branches.
- D46/D47/D48 unchanged (vendor ROM behavior, the 300 s `onTimeout` window with a *default* 6 h
  budget, and real-device confirmation) — none reproducible on these images.
- `ServiceCompat.startForeground()` is still not exercised; recommendation unchanged.

## 7. Environment state after this round

Both emulators I started are stopped and confirmed gone; nothing was left running or configured.

- `Pixel_6_API_29` (pid 4358, port 5554) and `Pixel_6_API_37` (pid 4359, port 5556), started
  16:51:40, both stopped via `adb emu kill`; `ps` afterwards shows no `qemu-system` and no
  `emulator -avd` process of mine (and none at all on the machine), `adb devices` is empty, and both
  pids are dead (`logs/b4-pids.txt`, `logs/b4-final-state.txt`).
- `com.example.fgslab`: uninstalled from both (`Success` + `pm list packages` empty on 37; the 29
  device's copy was already removed by `queue.sh`'s cleanup — a second uninstall reports
  `DELETE_FAILED_INTERNAL_ERROR` precisely because the package is gone, and `pm list packages` is
  empty).
- Every `device_config` key this work ever writes (`data_sync_fgs_timeout_duration`,
  `media_processing_fgs_timeout_duration`, `short_fgs_timeout_duration`) reads back `null` on both
  devices. This round set no key (the API 29 amendment queue is below the knob-capable levels and the
  one API 37 case is not a knob case).
- Config sync: `get_sync_disabled_for_tests` → `none` on API 37 (explicitly set back to `none`; it was
  already `none`), command does not exist on API 29 (recorded `Invalid command`).
- `Pixel_6_API_31` / `Pixel_6_API_34` were not started this round; their state is A4 §7. No AVD was
  deleted, renamed, wiped or recreated; `Pixel_10_Pro_XL` was not started. No Gradle build ran. No
  git state-changing command and nothing was posted.

**Verdict: 2 defects found and fixed (D58, D59), both left over in the round bookkeeping A4 rewrote: an artifacts line that mis-states which AOSP sources are saved, and a parenthetical claiming `apk_md5=` is in every `case.txt` when the 49 build-side cases predate that field. A4's own fixes (D54–D57) and all of its re-runs re-verified clean. The recommendation, its three amendments and every measured result stand. Not clean.**
