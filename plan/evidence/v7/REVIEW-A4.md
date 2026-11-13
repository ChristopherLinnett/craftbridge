# Review round A4 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: Claude, round A4, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md` and
`REVIEW-B3.md` as handed over after round B3, the earlier review files, `app/`, every APK set,
`evidence/`, `docs/`, `logs/`, the scripts. Method as before: new evidence only under
`CASE_TAG=review-a4` (APK directory passed to `queue.sh`), earlier evidence and review files untouched,
a re-run that agrees with earlier evidence apart from instance values is not a defect and does not
change RESULT.md.

**Verdict: 4 defects found and fixed (D54–D57): two citation-fidelity errors (an unmarked omission in a source block, two quotes missing class names), a source description that left out a feature flag, and stale round bookkeeping. The recommendation, its three amendments and every measured result stand. Not clean.**

## 1. B3 claims and fixes checked

| B3 claim / fix | How checked | Result |
|---|---|---|
| D53: the android14 `defaultFgsTypes` block now matches `docs/` verbatim | extracted the fenced block from RESULT.md, stripped indentation, `diff` against `docs/aosp-android14-release-ActiveServices.java` lines 2311–2318 | identical (comment lines included) ✓ |
| D53: `will default to "none"` | `docs/aosp-android14-release-ForegroundServiceTypePolicy.java` line 1335 | verbatim ✓ |
| §3: the android15 `onFgsTimeout` block "matches … verbatim including its `…` elision" | same extraction, then each block line located in lines 3900–3909 of `docs/aosp-android15-release-ActiveServices.java` in order | **false**: the block goes from the `if` line straight to the comment, skipping four source lines (`// Discard any other messages for this service` and the two `removeMessages` calls) with no `…`; only the later gap was marked → D54 |
| §3: the prose citations around the blocks check out | a script that takes every double-quoted passage in RESULT.md (57 per line, 60 per paragraph, fenced blocks excluded), normalises markup and whitespace, splits at `…`, and looks every segment up in all of `docs/*.txt` and `docs/*.java` | around the blocks, yes. Elsewhere two quotes are not on their pages as quoted: they drop the `ServiceInfo.` and `Build.VERSION_CODES.` qualifiers that the pages print (checked in `docs/service-ref.txt`, `docs/serviceinfo-ref.txt` and the rendered `docs/service-ref.html`) → D55. Every other quoted passage is found on the page it is attributed to; the remaining non-matches are not quotations (the manifest value `"mediaProcessing|dataSync"`, the scare-quoted "extra second" and "Swipe to dismiss from recents", the hypothetical "no type on old devices") |
| §3: the exhaustion exception is flag-gated by `enableFgsTimeoutCrashBehavior()`; "RESULT.md neither states nor relies on" it | read `docs/aosp-android15-release-ActiveServices.java` lines 2437–2466 and 3947–3980 | gating true, and the same flag also gates the crash after an unhandled `onTimeout` (line 3964, "Log a WTF instead of crashing the app while the new behavior is gated"), which B3 did not mention. RESULT.md *did* state the exception branch as unconditional ("… `ForegroundServiceStartNotAllowedException(…)` otherwise") → D56 |
| §1: all ~100 `case.txt` `apk_md5=` values match RESULT.md's per-round build list | read the list | true when written, but the list (and the header, end-state pointer, artifacts and label lists) did not include round B3 → D57 |
| §2: 15 `review-b3-*` cases agree with A3 | own re-runs (§2) and spot checks in B3's evidence: `review-b3-a29-combo-auto-long` `typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=417280`, `review-b3-a31-combo-auto-long` `fileKiB=416128` (both `apk_md5=3d45b867…`, device SDKs 29/31); `short-notdeclared` +104 ms (29) and +166 ms (31) with `fatal_exception_at_launch=` set; `review-b3-a29-combo-mediaproc` has no crash field while its log holds a foreign `FATAL EXCEPTION` (pid 3633, own pid 3701); `review-b3-harness-a31-relabel-b3` holds only `am_start`, `case`, `install`, `logcat_full`; `crash-then-ok-1` records its `RemoteServiceException`, `-2` (pid 5284, `EXPORT_DONE`) records none | all as B3 states |
| §7: B3's end state on 29/31 | my boot of 29 found no test app; my 29 and 37 read-backs (§7) | consistent |

## 2. Cases re-run (review A4)

Emulators started this round, tracked by pid (`logs/a4-pids.txt`): `Pixel_6_API_29` 73755 (port 5554)
and `Pixel_6_API_37` 73756 (port 5556), both at 16:25:08; never a third. All cases ran `apk-b1/` (combo
`3d45b867…`): `queue.sh emulator-5554 29 amendments /tmp/craftbridge-v7/apk-b1` with
`CASE_TAG=review-a4`, and one `run_case.sh` call on 37. 7 new directories, all `review-a4-*`.

| Case | Device | Result | Against earlier evidence |
|---|---|---|---|
| `review-a4-a29-combo-auto-long` | 29 | `STARTFOREGROUND_OK requested=auto typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=420864`; `dumpsys_bg.txt` `isForeground=true`, launcher resumed, `curProcState=5`; `dumpsys_late.txt` `(nothing)` | agrees with `review-a3-`/`review-b3-a29-combo-auto-long` (420224 / 417280) |
| `review-a4-a29-combo-short-notdeclared` | 29 | genuine crash: `IllegalArgumentException … not a subset … 0x00002001` → `android.app.RemoteServiceException` +41 ms, `fatal_exception_at_launch=android.app.RemoteServiceException` | agrees; 41 ms is an instance value inside RESULT's "within a second" |
| `review-a4-a29-combo-{none,mediaproc,datasync}` | 29 | ✅ `typeBit=0`, ✅ `8192`, ✅ `1`; no crash field on any (the mediaproc case follows the crash case) | agree |
| `review-a4-a29-combo-bad-type-name` | 29 | `am start -W` bounded, `APP_CRASHED`, `RuntimeException;IllegalArgumentException` | agrees |
| `review-a4-a37-combo-short-notdeclared` | 37 | genuine crash: not-a-subset → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` +77 ms | agrees with `a37-`/`review-b1-`/`review-a2-a37-combo-short-notdeclared` |

The 37 boot also served the flag check (§3, `logs/a4-flag37.txt`). No re-run disagreed with earlier
evidence; none changed RESULT.md.

## 3. Documentation and source read

* `docs/aosp-android14-release-ActiveServices.java` 2300–2330 (the per-bit loop and the `defaultFgsTypes`
  block, inside `setServiceForegroundInnerLocked`, which starts at 1993; `validateForegroundServiceType`
  at 2552 maps `FGS_TYPE_POLICY_CHECK_DISABLED` to `InvalidForegroundServiceTypeException … has been
  prohibited`); `docs/aosp-android14-release-ForegroundServiceTypePolicy.java` (`FGS_TYPE_POLICY_NONE`
  with `FGS_TYPE_NONE_DISABLED_CHANGE_ID`, `@EnabledAfter(targetSdkVersion = TIRAMISU)`, i.e. target ≥ 34;
  the fallback at 1347–1356; the `will default to "none"` comment at 1335, inside the
  `DefaultForegroundServiceTypePolicy()` constructor that starts at 1305). RESULT.md's description of
  these matches.
* `docs/aosp-android15-release-ActiveServices.java`: the start-time branch 2437–2466 (first condition:
  first start more than 24 h ago; then TOP; then, for an exhausted budget, `if
  (android.app.Flags.enableFgsTimeoutCrashBehavior())` throw, else `Slog.wtf` and reset — the comment at
  2457: "Only throw an exception if the new crashing logic gate is enabled; otherwise, reset the limit
  temporarily."); `onFgsTimeout` 3887–3945; `onFgsCrashTimeout` 3947–3980 with the same flag (crash, or
  "Log a WTF instead of crashing the app while the new behavior is gated").
* `docs/service-ref.txt` / `docs/service-ref.html` (`onTimeout(int)`: "… onTimeout(int, int) will also be
  called when a foreground service of type ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE times
  out."), `docs/serviceinfo-ref.txt` (`FOREGROUND_SERVICE_TYPE_DATA_SYNC`: "This type has time limit of 6
  hours starting from Android version Build.VERSION_CODES.VANILLA_ICE_CREAM.").
* On the API 37 image (`logs/a4-flag37.txt`): `aflags list` → "Error: must be root"; the flag name
  `enable_fgs_timeout_crash_behavior` occurs in none of `/system`, `/system_ext`, `/product`, `/vendor`
  `etc/aconfig_flags.pb` or `/system/etc/aconfig/*`, while other FGS flags (`fgs_delegate_system_api`,
  `asynchronous_fgs_logging_lms`, `always_revoke_audio_capability_above_bfgs`) are present in
  `/system/etc/aconfig_flags.pb`.

## 4. Defects found and fixed

Files changed this round: `RESULT.md`; new `REVIEW-A4.md`; new `evidence/review-a4-*` (7
directories); new `logs/a4-*` (queue and emulator logs, `a4-pids.txt`, `a4-flag37.txt`,
`a4-final-state.txt`). No app source or script change; no earlier evidence or review file changed.

* **D54 — the android15 `onFgsTimeout` block left out four lines without marking it.** Between the `if`
  line and the "The app was in the TOP state …" comment the source has `// Discard any other messages for
  this service` and two `removeMessages(…)` calls; the block showed the comment directly under the `if`,
  marking only the later gap with `…`. B3 checked this block and called it verbatim. Fix: a `…` line where
  the four lines are left out; a script now confirms every block line occurs in the source in order and
  every gap is marked.
* **D55 — two documentation quotes dropped the class names the pages print.** "will also be called when a
  foreground service of type FOREGROUND_SERVICE_TYPE_SHORT_SERVICE times out" (Service reference) and
  "has time limit of 6 hours starting from Android version VANILLA_ICE_CREAM" (ServiceInfo reference):
  both pages, as saved and as rendered, read `ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE` and
  `Build.VERSION_CODES.VANILLA_ICE_CREAM`. Same kind of error as D53, in prose B3 did not check. Fix:
  both quotes verbatim. After the fix every quotation in RESULT.md is found on its page.
* **D56 — RESULT.md described the exhausted-budget branch as unconditional.** It said the source throws
  `ForegroundServiceStartNotAllowedException("Time limit already exhausted …")` "otherwise"; in the
  android15-release copy it cites, the throw happens only if `android.app.Flags.enableFgsTimeoutCrashBehavior()`
  is on, and with the flag off the platform logs a WTF and resets the record. The same flag decides
  whether an `onTimeout` that is not followed by `stopSelf()` crashes the app. So the claim was wrong
  without the flag, and the flag belongs in RESULT.md where the source is described; it does not change
  any advice (stop within seconds, start from the foreground) or any measured result. Fix: the reset
  paragraph now gives the full branch structure (24 h, TOP, flag-gated throw or WTF + reset), names the
  second use of the flag with line numbers, and records that the API 37 image declares no such flag and
  `aflags` needs root there, so the flag's state on a given build was not established. "Not exercised"
  points to it.
* **D57 — RESULT.md's round bookkeeping went stale again with B3**, as it had with B2 (D49): the header
  listed rounds up to A3, the end-state pointer named `REVIEW-A3.md`, the per-round build list and the
  artifacts and label lists stopped at A3, and the `docs/` line listed neither B2's SettingsProvider
  sources nor the two cited pages that exist only as live reads. Fix, so that it does not need editing
  every round: the header, end-state pointer, build list, review-file entry and label list are written
  per round in general (`REVIEW-<round>.md`, "the newest file", "every `review-<round>-*` case from
  review B1 on ran `apk-b1/` except …"), and the `docs/` line lists every saved copy and the two
  live-only pages.

## 5. Agreement with the recommendation

Agree, unchanged: declare `mediaProcessing|dataSync` with `FOREGROUND_SERVICE`,
`FOREGROUND_SERVICE_MEDIA_PROCESSING` and `FOREGROUND_SERVICE_DATA_SYNC`, and pass `mediaProcessing`
when `SDK_INT >= 35`, else `dataSync`, with round A1's three amendments. The flag finding (D56) makes the
documented consequences of not stopping after `onTimeout`, and of a background start against a spent
budget, build-dependent in the source, which strengthens rather than changes the advice: stop within
seconds of `onTimeout`, stop when the export ends, start from the foreground. The re-runs on 29 and 37
agree with every earlier result.

## 6. Open items

* Carried over, unchanged: API 30/32/33 untested; the full 24 h window; a background start against an
  exhausted budget; `ServiceCompat.startForeground()`; an `onTimeout` handler that does not stop (now
  known to be flag-gated in the android15 source; the flag is not declared on the API 37 image and its
  state on Android 15/16 builds is not known here); everything in RESULT.md's "What the emulator cannot
  show" list.

## 7. Environment state after this round

Written after both emulators were stopped. `ps ax` shows no `qemu-system`, `emulator -avd` or emulator
`crashpad_handler` process, and `adb devices` is empty. Every emulator this round started is gone:
73755 (`Pixel_6_API_29`) and 73756 (`Pixel_6_API_37`), each stopped with `adb emu kill` and confirmed by
pid. Read back before each stop (`logs/a4-final-state.txt`): `com.example.fgslab` absent on 29
(uninstalled by `queue.sh`, `Success`) and on 37 (uninstalled, `Success`);
`data_sync_fgs_timeout_duration`, `media_processing_fgs_timeout_duration` and
`short_fgs_timeout_duration` `null` on both; `get_sync_disabled_for_tests` `none` on 37 and absent on 29.
This round set no `device_config` key and did not change the sync mode. No AVD deleted, renamed,
recreated or wiped; `Pixel_10_Pro_XL` not started; no Gradle build. Nothing was committed, pushed or
posted.

**Verdict: 4 defects found and fixed (D54–D57): two citation-fidelity errors (an unmarked omission in a source block, two quotes missing class names), a source description that left out a feature flag, and stale round bookkeeping. The recommendation, its three amendments and every measured result stand. Not clean.**
