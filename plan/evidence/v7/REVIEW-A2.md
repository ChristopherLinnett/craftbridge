# Review round A2 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: Claude, round A2, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md` and
`REVIEW-B1.md` as handed over after round B1, `REVIEW-A1.md`, `app/`, every APK set, `evidence/`,
`docs/` (incl. B1's AOSP copies), `logs/`, `run_case.sh`, `queue.sh`, `summarize.sh`. Method, as set for
this round: every re-run went into new evidence directories (`CASE_TAG=review-a2` and
`CASE_TAG=review-a2-guarded`); no earlier evidence was changed; a re-run that agrees with earlier
evidence apart from instance values is not counted as a defect and did not change RESULT.md (the rule is
now stated once at the top of RESULT.md). RESULT.md was changed only where a claim, number or citation
was wrong, missing or unsupported.

**Verdict: 14 defects found and fixed (D31–D44); the recommendation and its three amendments stand. One environment item is left for the user: `Pixel_6_API_29` no longer boots and needs a data wipe to repair.**

## 1. B1 claims and fixes checked

| B1 claim / fix | How checked | Result |
|---|---|---|
| `apk-b1*` rebuild byte-identical to `apk-a1*` | `md5` of all nine files; my own `./gradlew clean assembleDebug`, `-PtargetSdk=34`, `-PtargetSdk=33` (`logs/a2-build-*.log`) | identical (combo `3d45b867…`, full `8e76be90…`, none `16fad23e…`, noperm `15b2745f…`, ts34 ×4, ts33); 0 warning lines |
| 24 B1 scenarios ran on the right device with the right APK | per case: `apk_md5`, `device_sdk`, result, own-pid markers | all 26 non-void dirs match their labels; 26 installs `Success`, 1 void (`Can't find service: package`) as stated |
| subset-of-manifest rule on every level; three crash class names; 55–190 ms | `IllegalArgumentException … not a subset … 0x00002001` and the `FATAL EXCEPTION` class + timestamp in `review-b1-a{29,31,34,37}-combo-short-notdeclared`, `a34/a37-combo-short-notdeclared` | true: `RemoteServiceException` (29, +76 ms), `ForegroundServiceDidNotStartInTimeException` (31, +147 ms), `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` (34 +68/+186 ms, 37 +55/+98 ms); re-run on 34/37 (+192/+79 ms) agrees; on 31 +649 ms, outside the stated range → D41 |
| Android 14 unknown-bit rule from AOSP | read `docs/aosp-android14-release-ActiveServices.java` (per-bit loop with `defaultFgsTypes`, lines 2309–2329) and `…ForegroundServiceTypePolicy.java` (`// Unknown type, fallback to the defaultToType`, `will default to "none"`) | quotes exact; explanation matches `a34-full-none` vs `review-*-a34-combo-none` and `a34-noperm-mediaproc` |
| 6-hour clock resolved from `ActiveServices.onFgsTimeout`, six runs at 61.0–61.7 s | source read; HOME and timeout both taken from device log lines | mechanism right, numbers and part of the explanation wrong → D37, D38 |
| exhaustion then foreground start "confirms the documented reset" | source (`fgsTypeInfo.reset()` branch, `getNextFgsStopTime`, `onFgsTimeout`); re-run | overclaim → D39 |
| throughput ≈630 KiB/s (D19) | `TICK elapsedS=60 fileKiB=38208`, 417,280 KiB / 660 s | true |
| Play Console and wake-lock quotes now verbatim (D26, D27) | compared with `docs/play-console-fgs.txt`, `docs/awake.txt` | not verbatim / misattributed, and a spliced leftover → D34, D35 |
| `run_case.sh` records `fatal_exception_at_launch=` and `target_sdk=` (D29) | read; ran it | works, but duplicated, unscoped, missing on the `APP_CRASHED` path → D44; RESULT claims all B1 cases have `target_sdk=` → D33 |
| `queue.sh` `CASE_TAG` + `device_config` read-backs (D30) | read; ran it with `CASE_TAG=review-a2` on 34 and 37 | works; B1's own extra scenarios lived outside it in copied scripts → D43; read-back after delete cannot detect a key lost mid-run → D42 |
| §7: all emulators stopped, `adb devices` empty | `ps` at the start of A2 (12:12) | **false**: a `Pixel_6_API_29` qemu (pid 29106, started 11:00:07, `adb protocol fault`, invisible to `adb devices`) was still running → D31 |
| A1's 18 defects still hold | spot checks: void note, `onTimeout` naming, `review-a1-build-compare/` | agree |

## 2. Cases re-run (review A2)

All A2 cases ran `apk-b1/` (combo `3d45b867…`, full `8e76be90…`) through `queue.sh` with
`CASE_TAG=review-a2` (set `amendments`, 31 cases on 31/34/37) and `CASE_TAG=review-a2-guarded` (set
`timeouts`, after the D42 fix). Every `case.txt` records `apk_md5`, `device_sdk`, `pid`,
`home_sent_device` and `target_sdk`; outcomes are from the case's own pid. 31 directories, all new; no
earlier directory was touched.

| Case(s) | Device | Result | Against earlier evidence |
|---|---|---|---|
| `review-a2-a34-combo-none` | 34 | ❌ `InvalidForegroundServiceTypeException: Starting FGS with type unknown … has been prohibited`, no `FATAL` | agrees |
| `review-a2-a34-combo-mediaproc` | 34 | ❌ same exception | agrees |
| `review-a2-a34-combo-short-notdeclared` | 34 | not-a-subset → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` +192 ms | agrees |
| `review-a2-a34-combo-datasync`, `-combo-auto-long` | 34 | ✅ 45 s; ✅ 660 s `fileKiB=419264`, bg dumpsys `types=00000001`, launcher resumed | agrees |
| `review-a2-a34-full-short-timeout`, `review-a2-guarded-a34-full-short-timeout` | 34 | `FGS_ON_TIMEOUT_1ARG` 180.03 s and 180.01 s after `STARTFOREGROUND_OK` | agrees |
| `review-a2-a34-combo-datasync-knob60` | 34 | ran 150 s, but the key was gone at the end (`Failed to delete`) | inconclusive → D42 |
| `review-a2-guarded-a34-combo-datasync-knob60` | 34 | sync suspended; key still `60000` at the end; ran its full 150 s | agrees with B1 |
| `review-a2-a34-combo-bad-type-name` | 34 | `APP_CRASHED`, `fatal_exception=java.lang.RuntimeException;java.lang.IllegalArgumentException` | agrees |
| `review-a2-a37-combo-none`, `-mediaproc`, `-datasync` | 37 | ✅ ✅ ✅ | agrees |
| `review-a2-a37-combo-short-notdeclared` | 37 | not-a-subset → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` +79 ms | agrees |
| `review-a2-a37-combo-auto-long` | 37 | ✅ 660 s `typeBit=8192`, `fileKiB=414464`, bg dumpsys `types=0x00002000` | agrees |
| `review-a2-a37-combo-auto-knob60`, `review-a2-guarded-a37-combo-auto-knob60` | 37 | `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192` 61.08 s after the HOME line, 60.02 s after `lastTopTime` | agrees; adds the `lastTopTime` measurement (D38) |
| `review-a2-a37-combo-datasync-knob60` | 37 | **no timeout**, ran 300 s: a flag sync removed the key 26 s after it was set (`SettingsToPropertiesMapper` at 12:19:41, `Failed to delete` after) | harness artefact → D42 |
| `review-a2-guarded-a37-combo-datasync-knob60` | 37 | sync suspended; `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=1` 61.06 s after HOME, 60.02 s after `lastTopTime`; key `60000` at the end | agrees with A1/B1 |
| `review-a2-a37-exhaust-{1st-timeout,2nd-foreground}`, `review-a2-guarded-a37-exhaust-{…}` | 37 | 1st times out; the foreground start right after is accepted and times out 60.02 s after its own `lastTopTime` (B1's 2nd run was 45 s, too short to time out) | agrees; source shows it cannot isolate the reset → D39 |
| `review-a2-a31-combo-none`, `-mediaproc`, `-datasync` | 31 | ✅ ✅ ✅ | agrees with B1 |
| `review-a2-a31-combo-short-notdeclared` | 31 | not-a-subset → `android.app.ForegroundServiceDidNotStartInTimeException`, **+649 ms** | class agrees; the delay is outside RESULT's "55–190 ms" → D41 |
| `review-a2-a31-combo-bad-type-name` | 31 | `APP_CRASHED`, `fatal_exception=java.lang.RuntimeException;java.lang.IllegalArgumentException` | agrees |
| API 29 | — | **not re-run**: `Pixel_6_API_29` no longer boots (see §6, §7) | — |

The 34/37 `short-notdeclared` cases were recorded before the D44 scoping fix, so their
`fatal_exception_at_launch=` also lists a stack-frame method name; the exception class is the first item.
Builds: `./gradlew clean assembleDebug`, `-PtargetSdk=34`, `-PtargetSdk=33` reproduce all nine `apk-b1*`
md5s with zero warning lines (`logs/a2-build-*.log`).

## 3. Documentation and source read (2026-11-13)

Re-read this round, live and against the copies in `docs/`:

* [Keep the device awake](https://developer.android.com/develop/background-work/background-tasks/awake) (live and `docs/awake.txt`) — the answer to the "detrimental … if the device suspends" question is: "If Yes, you might need to use a wake lock. However, you should still check to see if you're already using an API or doing an action that declares a wake lock on your behalf, as discussed in Actions that keep the device awake." That section opens "If your app is doing any of the following, you don't need to set a wake lock yourself." The sentence B1 quoted ("However, you should still check to see if any of the following, you don't need to set a wake lock yourself") is a splice of the two and is on neither copy.
* [Play Console: foreground service requirements](https://support.google.com/googleplay/android-developer/answer/13392821) (live and `docs/play-console-fgs.txt`) — `TYPE_DATA_SYNC` rows: "Network transfer : Backup and restore" carries "Use for specifically user-initiated features."; "Local processing : Import or export" carries "Includes migration of files from SD card." and the video "Show the user manually triggering an import or export of files locally"; "Local processing : Other" carries "Use for specifically user-initiated work and not for regular system or server-initiated tasks. For example, encryption, resizing."; the user-initiated-data-transfer-job recommendation is scoped to "transferring data over a network … in response to an explicit user request". `FOREGROUND_SERVICE_MEDIA_PROCESSING`: "Media transcoding", "Performing time-consuming operations on media assets, like converting media to different formats."
* AOSP android15 `ActiveServices.java` (`docs/aosp-android15-release-ActiveServices.java`): the start-time branch (lines 2437–2466) resets the type's record when "the app is currently in the TOP state or was in the TOP state after the time limit was exhausted previously" and otherwise throws `ForegroundServiceStartNotAllowedException("Time limit already exhausted …")`; `getNextFgsStopTime` (line 3773) arms the timer at `lastFgsStartTime + max(0, limit − totalRuntime)`; `onFgsTimeout` (line 3887) re-arms to `lastTopTime + limit` while the app was TOP within the last `limit`, else delivers the timeout. `ServiceRecord.TimeLimitedFgsInfo` (`docs/aosp-android15-release-ServiceRecord.java`) accumulates `mTotalRuntime` from each start regardless of process state. Consequences: (1) runtime is not "accumulated only while not TOP" — the TOP effect is the re-arm; (2) after a TOP start the timeout lands at `lastTopTime + limit` whether or not the record was reset, so a foreground restart cannot show the reset.
* AOSP android14 `ActiveServices.java` and `ForegroundServiceTypePolicy.java` — B1's quotes re-checked character for character (comment at lines 2311–2314, `getForegroundServiceTypePolicyInfo` fallback at 1347–1356).
* `device_config` (shell help on the devices): `set_sync_disabled_for_tests none|persistent|until_reboot` and `get_sync_disabled_for_tests`, used by the fixed knob helpers.

## 4. Defects found and fixed

Files changed this round: `RESULT.md`; `run_case.sh`; `queue.sh`; new `REVIEW-A2.md`; new
`evidence/review-a2-*` (31 directories) with `NOTE-review-a2.txt` in the two sync-affected knob runs;
new `logs/a2-*` (builds, emulator boots incl. the two API 29 boot-loop logcats, queue runs). No app source
change (my rebuild equals `apk-b1*`), no earlier evidence or review file changed.

**Environment**

* **D31 — B1 left an emulator running and reported the environment clean.** At 12:12 a `Pixel_6_API_29`
  qemu (pid 29106, started 11:00:07 = `logs/b1-emulator-29.log`, last line `adb protocol fault`) was still
  running, invisible to `adb devices`, contrary to REVIEW-B1 §7 and RESULT.md's "End state after review
  B1". With my two emulators it also made three. Fix: stopped it (SIGTERM, gone within seconds); RESULT.md's
  end-state line now points at the latest review's §7, and §7 below is checked with `ps`, not only
  `adb devices`. Consequence left open (§6): `Pixel_6_API_29` no longer boots.

**RESULT.md accuracy**

* **D32 — the verdict called `3d45b867…` "the byte-identical review-B1 rebuild" of `0cea776e…` and said it completed 660 s on all four levels.** The two md5s differ (A1 changed the app code; B1's rebuild is identical to `apk-a1`, not to `apk2`), and `3d45…` ran 660 s only on 34 and 37. The one-APK table also listed B1's `3d45…` runs under the header "Same file `apk2/…` md5 `0cea…`". Fix: both stated per APK.
* **D33 — "`review-b1-*` and every later run record `target_sdk=` too"** — only the two `review-b1-harness-smoke-*` cases do; the 24 B1 scenarios ran the earlier harness. Fix: says which cases have the field.
* **D34 — Play Console point: a spliced leftover and a misattributed quote.** After the new text the old paraphrase continued mid-word ("…for the video.description of the feature …"), and "for specifically user-initiated features" was attributed to the "Local processing : Import or export" row; on the page it belongs to "Network transfer : Backup and restore". Fix: point 9 rewritten from the page (live and `docs/play-console-fgs.txt`), with both local-processing rows and the network scope of the user-initiated-job recommendation.
* **D35 — the keep-awake "verbatim" quote does not exist.** "However, you should still check to see if any of the following, you don't need to set a wake lock yourself" splices two sentences. Fix: the two real sentences.
* **D36 — "an unknown future type bit in the manifest is only survivable next to `specialUse`"** is false as written: the recommended manifest with an explicit known type runs on 34 (every `auto` run there). It holds for the type-less call. Fix: says so.
* **D37 — the six "61.0–61.7 s after HOME" figures were computed from the host's second-truncated `home_sent=`** against device log times. On the device clock (HOME = `ActivityStartInterceptor: Starting home with component specified`) all six are 61.05–61.12 s. Fix: numbers and method in RESULT.md; `run_case.sh` now records `home_sent_device=` (the A1 API 29 run showed host and guest clocks can be minutes apart).
* **D38 — the 6-hour clock mechanism was mis-described.** "The allowance is accumulated only while the app is *not* in TOP" does not match the source (runtime accumulates from each start; the TOP effect is the re-arm in `onFgsTimeout`), and "the extra second being message scheduling" has no support: the message is posted for exactly `lastTopTime + limit`. Measured in seven A2 runs (`processes_bg.txt`): the timeout came 60.02 s after `lastTopTime`, and the app left TOP 1.04–1.06 s after the HOME line. Fix: paragraph rewritten with the source and the measurement.
* **D39 — "documented reset confirmed" / "The reset half is now exercised" overclaimed.** From TOP the start branch always resets the record, and the timeout of a TOP-started service is re-armed to `lastTopTime + limit` either way, so a foreground restart cannot show the reset; only a background start against an exhausted budget would. Fix: RESULT.md says what the runs show (accepted, fresh 60 s) and why that does not isolate the reset.
* **D40 — "The pre-review `app/` source rebuilds it byte-for-byte" had no artifact.** It was observed by the A1 reviewer and not logged, and that source is gone. Fix: labelled as such.
* **D41 — "the process dies 55–190 ms after the caught exception on every level"** was given as a bound; `review-a2-a31-combo-short-notdeclared` took 649 ms (the system tears the service down at once; the crash waits for the app's main thread). Fix: "within a second", with both figures.

**Harness**

* **D42 — `device_config` knobs could be lost mid-run, undetected.** On the Play-services images a flag sync ~7 minutes after boot (`SettingsToPropertiesMapper` storm) removed the local override: `review-a2-a37-combo-datasync-knob60` then never timed out, and `review-a1-a34-…`/`review-a2-a34-combo-datasync-knob60` ended with `Failed to delete`, so those two runs never showed the knob in force (B1's 34 run did: `Successfully deleted`). The read-back after deletion cannot notice this. Fix: `knob_on` runs `device_config set_sync_disabled_for_tests until_reboot` (the device help: "When in one of the disabled modes this ensures that config isn't overwritten"), `knob_off` reads the key back *before* deleting it, then deletes, reads back `null`, and restores `none`. Re-run as `review-a2-guarded-*`: every key `60000` at the end, every expected timeout fired. RESULT.md now names which 34 runs prove the "ignored on 14" claim.
* **D43 — `queue.sh` did not cover the cases B1 relied on, and had two relabel mechanisms.** B1 ran its 29/31/34/37 type-less/undeclared cases and the exhaustion pair from ad-hoc `logs/b1-run*.sh` and `logs/b1-reset37.sh`, the latter a copy of `QT`'s knob block; `SMOKE_PREFIX` and `CASE_TAG` both relabelled; `CASE_TAG` was not in the usage text. Fix: sets `amendments` and `timeouts` (one case list), `knob_on`/`knob_off` shared by `QT` and the new `QX` (exhaustion then foreground restart), `SMOKE_PREFIX` removed, usage documents `CASE_TAG`. The `logs/b1-*.sh` files stay as B1's transcripts. Verified by every `review-a2-*` run.
* **D44 — `run_case.sh` end fields.** The `target_sdk=` line was duplicated in two branches; `fatal_exception_at_launch=` took ±3 lines after the first `FATAL EXCEPTION` of any process (it also picked up stack-frame method names) and nothing was recorded on the `APP_CRASHED` path. Fix: one `record_end_fields` used by both exits, class names taken only from the exception line after `Process: com.example.fgslab` and its `Caused by:` lines; `APP_CRASHED` records `fatal_exception=` (e.g. `review-a2-a31-combo-bad-type-name`). Also new: `processes_bg.txt` (process state incl. `lastTopTime`, with the device time).

## 5. Agreement with the recommendation

Agree, unchanged: declare `mediaProcessing|dataSync` with `FOREGROUND_SERVICE`,
`FOREGROUND_SERVICE_MEDIA_PROCESSING` and `FOREGROUND_SERVICE_DATA_SYNC`, and pass `mediaProcessing`
when `SDK_INT >= 35`, else `dataSync`, with round A1's three amendments. A2's re-runs agree with every
earlier conclusion on 31, 34 and 37:

1. **Explicit type mandatory** — the type-less start is rejected on 34 and accepted on 31 and 37; a type
   outside the manifest kills the app on all three, whatever it catches.
2. **Time limits only from Android 15** — on 34 a 60 s budget that verifiably stayed in force did nothing;
   on 37 both types timed out exactly `limit` after the app last left the foreground, and `stopSelf()` in
   `onTimeout(startId, fgsType)` ended them without a crash. The clock detail (D37, D38) changes no advice:
   stop the service when the export ends.
3. **Two Play Console declarations + wake-lock check** — confirmed against the live pages; the
   corrections (D34, D35) sharpen the wording and add the "Local processing : Other" row as a candidate
   for 10–14.

None of the 14 defects touches the recommendation; they are evidence, wording and harness defects.

## 6. Open items

* **`Pixel_6_API_29` does not boot any more.** Two cold boots in A2 (`logs/a2-emulator-29*.log`,
  `logs/a2-emulator-29-bootloop-logcat.txt`, `…-29b-bootloop-logcat.txt`) loop in system_server:
  `java.lang.RuntimeException: There must be exactly one installer; found []` at
  `PackageManagerService.getRequiredInstallerLPr`, 222 and 90 crashes, `sys.boot_completed` never set.
  The package state on its user data is damaged. Which event did it cannot be told from here: B1's
  first boot of that AVD (11:00) failed and that instance hung (D31), B1 later saw
  `Can't find service: package` on it, and I stopped the hung instance with SIGTERM. The repair is
  `emulator -avd Pixel_6_API_29 -wipe-data`, which erases that shared AVD's user data (other projects'
  installed apps and state). I did not do it; it needs the user's decision. Until then, whether
  `com.example.fgslab` is still installed on it cannot be checked, and the API 29 claims rest on the
  earlier rounds' evidence (A1, B1) plus the android10 source (`docs/aosp-android10-release-ActiveServices.java`
  has the not-a-subset check); A2 re-ran nothing on 29.
* Carried over from B1, still open: API 30, 32, 33 untested; the 24 h window and the background-start
  `ForegroundServiceStartNotAllowedException` against an exhausted budget; `ServiceCompat.startForeground()`;
  everything in RESULT.md's "What the emulator cannot show" list.

## 7. Environment state after this round

Checked at 12:56 with `ps` (no `qemu-system`, emulator `crashpad_handler`, `run_case.sh` or `queue.sh`
process) and `adb devices` (empty). Emulators this round started: `Pixel_6_API_34`, `Pixel_6_API_37`,
`Pixel_6_API_31` and twice `Pixel_6_API_29` (boot loop), never more than two at once after the B1
leftover was stopped; all stopped with `adb emu kill`. `com.example.fgslab`: absent on 31, 34 and 37
(`pm list packages` empty before each shutdown); on 29 not checkable (does not boot, §6).
`device_config`, namespace `activity_manager`: `data_sync_fgs_timeout_duration`,
`media_processing_fgs_timeout_duration` and `short_fgs_timeout_duration` read back `null` on 34 and 37,
`data_sync_fgs_timeout_duration` `null` on 31 (no key was set there); `get_sync_disabled_for_tests`
reads `none` on 34 and 37 (31 has no such command and was never changed). No AVD deleted, renamed,
recreated or wiped; `Pixel_10_Pro_XL` not started. My Gradle and Kotlin daemons (started 12:14–12:15)
are stopped; the three daemons started at 12:43–12:44 are not mine and were left alone. Nothing was
committed, pushed or posted.

**Verdict: 14 defects found and fixed (D31–D44), none of them in the recommendation; not clean. Open for the user: `Pixel_6_API_29` boot-loops and its repair (`-wipe-data`) would erase that shared AVD's data.**
