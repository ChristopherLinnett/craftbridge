# Review round A1 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: Claude, round A1, working alone in `/tmp/craftbridge-v7`. Inputs: the brief
(`/tmp/craftbridge-amend/v7-brief.md`, read only), `RESULT.md` as handed over, `app/`, the APK sets,
`evidence/`, the scripts. Every runtime statement below cites an evidence directory; every
documentation statement cites the page it was read from on 2026-11-13.

**Verdict: 18 defects found and fixed (none left open); the recommendation stands, with three amendments now in RESULT.md (section 5).**

## 1. Claims checked

For every case directory I tabulated the APK path from `case.txt`, the device SDK and target SDK from
the case's own `SERVICE_CREATED` line, and the outcome marker, then checked that the outcome line comes
from the case's own process (pid of its `ACTIVITY_STARTED_FGS_FROM_FOREGROUND` line). All 49
build-side cases: device SDK matches the `a<api>-` label; `install.txt` is `Success` in all 49. APK
md5s: `evidence/apk-md5.txt` listed only `apk2/`, although the whole `a29-*`/`a31-*` matrix (except
`*-combo-final`) ran `apk/`; the `apk/` files are dated 01:15, before those runs (01:20 on), so their
current md5s are the md5s that ran, and they are now recorded.

| Claim in RESULT.md (as handed over) | Evidence | Verdict |
|---|---|---|
| `a29-noneflavor-long` ✅(660) `EXPORT_DONE type=none elapsedS=660` | case lasted 01:38:26 → 01:38:44 (18 s). The only `EXPORT_DONE` in its log is `01:38:24.684 … ( 6332): EXPORT_DONE type=auto elapsedS=660`, from the previous case's process (`a29-combo-auto-long`, pid 6332); the case's own process (6861) logged only `STARTFOREGROUND_OK` and one `TICK` | **False** — stale log line; re-run as `review-a1-a29-noneflavor-long`: `EXPORT_DONE type=none elapsedS=660` from its own process — the conclusion holds, the old evidence did not show it |
| a37 shortService: `FGS_ON_TIMEOUT fgsType=1 timeoutSource=2048` | the platform signature is `onTimeout(int startId, int fgsType)` ([Service](https://developer.android.com/reference/android/app/Service#onTimeout(int,%20int))); the app had renamed the parameters, so the line means startId 1, fgsType 2048 (shortService) | **Misread** (app defect D3) |
| "On 34 devices with target ≥ 34 [dataSync] carries the same 6 h/24 h budget as mediaProcessing" | docs: limit "starting from Android version VANILLA_ICE_CREAM" ([ServiceInfo](https://developer.android.com/reference/android/content/pm/ServiceInfo#FOREGROUND_SERVICE_TYPE_DATA_SYNC)); device: `review-a1-a34-combo-datasync-knob60` (60 s budget ignored, change id unknown) | **False** |
| "a declared-but-unused type costs nothing at runtime" | `review-a1-a34-combo-none`: type-less start with the combo manifest on 34 → `InvalidForegroundServiceTypeException … type unknown … has been prohibited` | **False** for the type-less call |
| dataSync "45 s spot checks and a 660 s backgrounded export per level"; "(45 s and 660 s per level tested: 29/31/34 + 37)" | 660 s dataSync on 29/31/34 only (`*-combo-auto-long`, `auto`=dataSync); on 37 `auto` = mediaProcessing, dataSync ran 45 s (`a37-full-datasync`) | **Overstated** |
| shortService "no timeout on API 29" (`a29-full-short`) | the run lasted 45 s, a quarter of the 180 s limit | **Not shown by the evidence** (documented only); re-run `review-a1-a29-full-short-240`: ran 240 s, `TICK elapsedS=180` then `EXPORT_DONE elapsedS=240`, no timeout — claim now holds for 29 |
| "proven by the byte-identical `a29/a31-full-mediaproc` full-flavor runs (bit accepted) plus combo installs" | those ran `apk/fgslab-full-ts37.apk` (md5 f5b2…), not byte-identical to any combo APK | **False** wording |
| "differs only by a debug logging override added mid-run" (`apk/` vs `apk2/`) | `dexdump -d` of all `com.example.fgslab` classes, addresses stripped: the only difference is the added `onTimeout(I)V`; combo binary manifests byte-identical (`cmp`); both in `evidence/review-a1-build-compare/` | **True** (now named precisely) |
| Timeouts table: "timeout page warns that on pre-15 devices 'the system immediately caches the app'" | that note is in the Media processing section of [service-types](https://developer.android.com/develop/background-work/services/fgs/service-types) and the [Android 15 FGS-types page](https://developer.android.com/about/versions/15/changes/foreground-service-types); the [timeout page](https://developer.android.com/develop/background-work/services/fgs/timeout) has no such text | **Misattributed** |
| specialUse justification "is reviewed when you submit your app in the Google Play Console" | page text: "These values and corresponding use cases are reviewed when you submit your app in the Google Play Console." | **Misquoted** (meaning right) |
| Recommended snippet `ServiceCompat.startForeground(...)` | the app calls the platform `Service.startForeground(id, n, type)`; no androidx dependency | **Not what was tested** |
| "a combined declaration dilutes Play review" | no source; Play Console requires one declaration per declared type ([Play Console help](https://support.google.com/googleplay/android-developer/answer/13392821)) | **Unsupported**; replaced by the actual requirement |
| "Implement both `onTimeout` overloads" | reference: developers "do not need to implement both of the callbacks" on Android 15+; the one-argument one serves only shortService | **Over-broad**; narrowed |
| One-APK proof: same `apk2/` combo (0cea…) on 29/31/34/37, `EXPORT_DONE elapsedS=660` | `a29-combo-final` (sdk 29, 02:38→02:49), `a31-combo-final` (sdk 31), `a34-combo-auto-long` (sdk 34, fileKiB 413632), `a37-combo-auto-long` (sdk 37, typeBit 8192, fileKiB 408256); all ran `apk2/` | **True**; RESULT.md was written at 02:39, before the 29/31 runs ended at 02:49 — the stated results held |
| `mediaProcessing` rejected on 34 (full, ts34 full) | `a34-full-mediaproc`, `a34-full-ts34-mediaproc` (sdk 34, targetSdk 37/34) | **True**; re-run with combo |
| `dataSync` accepted on 34 | `a34-full-datasync`, `a34-combo-auto-long` | **True**; re-run |
| shortService timed out at exactly 180 s on 34 and 37 | `01:54:59.821 → 01:57:59.829` and `01:55:28.103 → 01:58:28.109`; `ActivityManager: Short FGS timed out`; no `ANR in` | **True**; re-run on 34 |
| untyped FGS throws on 34/37 for target ≥ 34; ts33 passes | `a34-noneflavor-none`, `a34-none-ts34-none` (Missing…Exception), `a34-none-ts33-none` (OK, targetSdk 33), `a37-noneflavor-none` | **True** |
| noperm exceptions (dataSync/specialUse `SecurityException`, mediaProcessing on 34 `Invalid…`) | `a34-noperm-*`, `a37-noperm-*` | **True** |
| `shortService` not in combo → `IllegalArgumentException … not a subset … 0x00002001` | `a34/a37-combo-short-notdeclared` | **True**, but incomplete: the same logs show the app then crashing with `ForegroundServiceDidNotStartInTimeException` (D14) |
| ts34 build accepted for mediaProcessing on 37 | `a37-full-ts34-mediaproc` (`targetSdk=34`, `typeBit=8192`, EXPORT_DONE) | **True** |
| specialUse 660 s on 34/37; 45 s on 29/31 | `a34/a37-full-specialuse-long`, `a29/a31-full-specialuse` | **True** |
| a31 untyped 660 s | `a31-noneflavor-long` own pid 8154, `EXPORT_DONE type=none elapsedS=660` | **True** |
| installs succeed with unknown bits/permissions/property | 49× `Success` | **True** |
| AVD names/images | `~/.android/avd/Pixel_6_API_*.avd/config.ini`: `hw.device.name=pixel_6`, images as listed; 37 fingerprint `…:17/CP41.260828.004.A7…:user` | **True** |
| quotes: shortService "about 3 minutes"; dataSync uses "Import or export operations"/"Local file processing"; timeout page "If an app targets Android 15 or higher…"/"a total of 6 hours in a 24-hour period"; MEDIA_PROCESSING "Added in API level 35"; "it can be also used on prior android versions"; `onTimeout(int,int)` "if it fails to do so, the app will crash"; background start | re-read on the pages (section 3) | **True** |

## 2. Cases re-run (review A1)

Harness: the fixed `run_case.sh` (all log buffers cleared, outcome matched on the case's own pid,
`apk_md5`/`device_sdk`/`pid` in `case.txt`, `dumpsys_bg.txt` + `resumed_bg.txt` 30 s after HOME),
driven by `review-a1-queue.sh` and then `queue.sh` (which replaced it). Emulators booted headless,
`-no-snapshot`, ports 5554/5556, never more than two at once.

All `review-a1-a*` cases ran the build side's final APKs (`apk2/`: combo `0cea776e…`, full
`c84df81a…`, none `2d6d8586…`); the `review-a1-fixed-a*` cases ran `apk-a1/`, the rebuild of `app/`
after the fixes in section 4 (combo `3d45b867…`). Each `case.txt` records `apk_md5` and
`device_sdk`, and each outcome below is from the case's own process.

| Case | Device | Result | Agrees with build side? |
|---|---|---|---|
| `review-a1-a34-combo-mediaproc` | 34 | ❌ `InvalidForegroundServiceTypeException: Starting FGS with type unknown … targetSDK=37 has been prohibited` | yes (`a34-full-mediaproc`) |
| `review-a1-a34-combo-datasync` | 34 | ✅ 45 s; `dumpsys_bg.txt` `isForeground=true … types=00000001`, launcher resumed | yes |
| `review-a1-a34-combo-auto-long` | 34 | ✅ `09:26:53.725 STARTFOREGROUND_OK requested=auto typeBit=1` → `09:37:53.824 EXPORT_DONE elapsedS=660 fileKiB=416512`; HOME at 09:27:01; bg dumpsys `types=00000001` | yes (`a34-combo-auto-long`) |
| `review-a1-a34-full-short-timeout` | 34 | `09:20:44.751 STARTFOREGROUND_OK … typeBit=2048` → `09:23:44.754 ActivityManager: Short FGS timed out` → `FGS_ON_TIMEOUT_1ARG startId=1` → `EXPORT_ABORTED elapsedS=180`; no ANR | yes |
| `review-a1-a34-combo-none` | 34 | ❌ type-less `startForeground` with the combo manifest: `InvalidForegroundServiceTypeException: Starting FGS with type unknown … has been prohibited` | **new** — contradicts "a declared-but-unused type costs nothing" |
| `review-a1-a34-combo-datasync-knob60` | 34 | `device_config … data_sync_fgs_timeout_duration 60000` read back as 60000; `FGS_INTRODUCE_TIME_LIMITS` absent from `platform_compat`, `am compat enable` → `Unknown or invalid change`; dataSync ran its full 150 s in the background (`EXPORT_DONE elapsedS=150`) | **new** — contradicts "6 h budget on 34" |
| `review-a1-a37-combo-mediaproc` | 37 | ✅ 45 s, `typeBit=8192`, bg dumpsys `types=0x00002000` | yes |
| `review-a1-a37-combo-datasync` | 37 | ✅ 45 s | yes |
| `review-a1-a37-combo-none` | 37 | ✅ 45 s (type-less start accepted on 37) | new |
| `review-a1-a37-combo-auto-long` | 37 | ✅ `09:27:20.276 STARTFOREGROUND_OK requested=auto typeBit=8192` → `09:38:20.286 EXPORT_DONE elapsedS=660 fileKiB=409664`; bg dumpsys `types=0x00002000`, launcher resumed | yes (`a37-combo-auto-long`) |
| `review-a1-a37-combo-auto-knob60` | 37 | budget 60 s; `platform_compat` `FGS_INTRODUCE_TIME_LIMITS; enableSinceTargetSdk=35`; `09:24:01.849 STARTFOREGROUND_OK typeBit=8192`, HOME 09:24:10.0, `09:25:11.030 ActivityManager: FGS (mediaProcessing) timed out` → `onTimeout(startId=1, fgsType=8192)` → `stopSelf()` → `EXPORT_ABORTED elapsedS=69`; no crash | new — first observation of the 15+ limit |
| `review-a1-a37-combo-datasync-knob60` | 37 | budget 60 s; HOME 09:25:49, `09:26:50.332 FGS (dataSync) timed out` → `onTimeout(startId=1, fgsType=1)` → `EXPORT_ABORTED elapsedS=69`; no crash | new |
| `review-a1-fixed-a34-combo-auto` | 34 | ✅ 45 s `typeBit=1` | fixed build behaves the same |
| `review-a1-fixed-a34-combo-mediaproc` | 34 | ❌ same `InvalidForegroundServiceTypeException` | same |
| `review-a1-fixed-a34-combo-bad-type-name` | 34 | `Caused by: java.lang.IllegalArgumentException: unknown type name: bogus` → `PROCESS_GONE_AT_LAUNCH` | the D16 fix: a harness typo now crashes instead of posing as a platform rejection |
| `review-a1-fixed-a37-combo-auto` | 37 | ✅ 45 s `typeBit=8192` | same |
| `review-a1-fixed-a37-combo-mediaproc` | 37 | ✅ 45 s | same |
| `review-a1-fixed-a37-combo-bad-type-name` | 37 | crash `unknown type name: bogus` → `PROCESS_GONE_AT_LAUNCH` | as on 34 |
| `review-a1-fixed-a37-combo-auto-knob60` | 37 | `09:43:39.052 STARTFOREGROUND_OK typeBit=8192`, HOME 09:43:47.2, `09:44:48.300 FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192` → `EXPORT_ABORTED elapsedS=69` | the D3 fix: the callback now logs the platform's parameter meaning |
| `review-a1-a29-full-short-240` | 29 | shortService ran 240 s: `TICK elapsedS=180`, `EXPORT_DONE type=shortService elapsedS=240`; no `onTimeout`, no ANR; bg dumpsys `isForeground=true`, launcher resumed | **new** — the build side's "no timeout on 29" now has evidence |
| `review-a1-fixed-a29-combo-auto` | 29 | ✅ 45 s `typeBit=1` | same |
| `review-a1-fixed-a29-combo-mediaproc` | 29 | ✅ 45 s `typeBit=8192` (no type checks on 29) | same as `a29-full-mediaproc`, now with the combo APK |
| `review-a1-fixed-a29-combo-bad-type-name` | 29 | `am start -W` never returned (`am_start_wait=gave_up_after_30s`), the process stayed alive behind its crash dialog, `Caused by: java.lang.IllegalArgumentException: unknown type name: bogus` → `APP_CRASHED` | the D2 crash-handling fix on 29 |
| `review-a1-a29-noneflavor-long` | 29 | untyped (none flavor) `STARTFOREGROUND_OK typeBit=0` → `EXPORT_DONE type=none elapsedS=660 fileKiB=421504`; bg dumpsys `isForeground=true`, launcher resumed | replaces the void `a29-noneflavor-long`; same conclusion |

All three shortened-budget runs on 37 (two mediaProcessing, one dataSync) timed out 61 s after HOME
and 69 s after `startForeground`:
the clock ran from the move to the background, as the timeout page describes, not from the start of
the service, as the `Service.onTimeout(int,int)` reference describes. Every `device_config` key set
here was removed again: both devices read back `null` at the end (on 34 `device_config delete` printed
`Failed to delete`, but the key reads back `null`).

Process notes for this round: (1) my first `bad-type-name` runs on 34/37 ended `WAIT_TIMEOUT` and the
one on 29 hung in `am start -W`; those exposed the crash-handling gaps fixed under D2 and were re-run.
(2) The first `review-a1-a29-noneflavor-long` was cut short by me, not by Android: after a host stall
the API 29 guest clock ran about 4 minutes behind the host, so the harness deadline would have
force-stopped the export before its device-time end; I stopped the harness, which let its chained
`adb uninstall` remove the app at 535 s while the service was still foreground and exporting
(`evidence/review-a1-a29-noneflavor-long-aborted/`, last line `TICK elapsedS=540`). The case was then
re-run in full.

## 3. Documentation read (2026-11-13)

* [FGS timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout) — applies when an app "targets Android 15 or higher", to time spent "while your app is in the background"; dataSync and mediaProcessing get "a total of 6 hours in a 24-hour period", the limit is "tracked separately for `dataSync` and for `mediaProcessing`" and shared by all services of a type; at the limit the system calls `Service.onTimeout(int, int)` "(introduced in Android 15)" and the service "has a few seconds to call `Service.stopSelf()`", otherwise `RemoteServiceException: "A foreground service of type [service type] did not stop within its timeout"`; the timer resets "if the user brings the app to the foreground"; starting another dataSync FGS with the budget spent throws `ForegroundServiceStartNotAllowedException` ("Time limit already exhausted…"). Testing: `adb shell am compat enable FGS_INTRODUCE_TIME_LIMITS`, `device_config put activity_manager data_sync_fgs_timeout_duration …` / `media_processing_fgs_timeout_duration …` — used here.
* [ServiceInfo](https://developer.android.com/reference/android/content/pm/ServiceInfo) — `FOREGROUND_SERVICE_TYPE_DATA_SYNC` (API 29, value 1): "time limit of 6 hours starting from Android version VANILLA_ICE_CREAM"; before that `onTimeout(int,int)` "will never be called", so developers "must make sure to stop the foreground service". `FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING` (API 35, value 8192): "it can be also used on prior android versions" — contradicted on Android 14 by the device. `SHORT_SERVICE`: "3 minute timeout"; combined with other types "the system will simply ignore it". `FOREGROUND_SERVICE_TYPE_MANIFEST` (-1): "use all types set in manifest file".
* [Service](https://developer.android.com/reference/android/app/Service) — `onTimeout(int startId, int fgsType)`, API 35: a grace period "of a few seconds … if it fails to do so, the app will crash"; the clock "begins as soon as the foreground service starts" and resets every 24 h or "if the app comes into the foreground state". `onTimeout(int startId)`, API 34, shortService only; ANR "after a short grace period of several seconds"; on 35+ the two-argument one is also called and developers "do not need to implement both". `startForeground(int, Notification, int)`: type "must be a subset" of the manifest attribute (else `IllegalArgumentException`); throws `ForegroundServiceStartNotAllowedException` (target ≥ 31, background), `InvalidForegroundServiceTypeException`, `MissingForegroundServiceTypeException`, `SecurityException` (target ≥ 34).
* [Service types](https://developer.android.com/develop/background-work/services/fgs/service-types) — Data sync: permission `FOREGROUND_SERVICE_DATA_SYNC`, uses include "Import or export operations" and "Local file processing"; targets 35+ may not start it from `BOOT_COMPLETED`; Alternatives → data-transfer guide. No deprecation notice. Media processing: "time-consuming operations on media assets, like converting media to different formats", 6 of every 24 hours; on timeout without `stopSelf()` "an ANR will occur"; note that on Android 14 or lower a timed-out media processing service is cached immediately. Short service: "about 3 minutes"; no callback and no ANR on Android 13 and lower. Special use: the `<property>` values "are reviewed when you submit your app in the Google Play Console". Play: apps targeting 14+ declare their types in Play Console (Policy > App content).
* [Data-transfer options](https://developer.android.com/about/versions/15/changes/datasync-migration) (the "Alternatives to data sync" link) — table row: `mediaProcessing` for "Encoding or decoding a media file"; user-initiated data-transfer jobs for user-started transfers; mediaProcessing "is available if your app targets Android 15 or higher" (device: a ts34 build is accepted on 37).
* [Changes to foreground services](https://developer.android.com/develop/background-work/services/fgs/changes) — Android 15 (target 35): dataSync time limits, similar for mediaProcessing; Android 16: jobs started from an FGS obey job quotas; nothing listed for Android 17.
* [Android 14 FGS types required](https://developer.android.com/about/versions/14/changes/fgs-types-required) — target 34 must declare types + per-type permission "in addition to" `FOREGROUND_SERVICE`; missing type → `MissingForegroundServiceTypeException`; best practice `ServiceCompat.startForeground` (androidx-core 1.12+); a call without a type "defaults to the values defined in the manifest", and with several types the service must meet the requirements "of **all** types". Nothing on a type the platform does not know: that behaviour is known only from the device (rejected when passed explicitly; type-less start rejected with `mediaProcessing|dataSync`, accepted with `dataSync|mediaProcessing|specialUse|shortService`).
* [Android 15 FGS-type changes](https://developer.android.com/about/versions/15/changes/foreground-service-types) — introduces mediaProcessing; dataSync not startable from `BOOT_COMPLETED` (target 35).
* [Background-start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start) — "Apps that target Android 12 (API level 31) or higher can't start foreground services while the app is running in the background, except for a few special cases" → `ForegroundServiceStartNotAllowedException`; exemptions include the app transitioning from a visible activity and the user acting on its UI.
* [FGS troubleshooting](https://developer.android.com/develop/background-work/services/fgs/troubleshooting) — dataSync/mediaProcessing overrun: "internal `RemoteServiceException` causing the app to crash"; `ForegroundServiceDidNotStartInTimeException` if `startForeground` is not called within seconds.
* [Play Console: foreground service requirements](https://support.google.com/googleplay/android-developer/answer/13392821) — targets 14+: for each type declared, a description, the user impact if deferred/interrupted, a video link, a use case; dataSync "specifically user-initiated", recommends the user-initiated data transfer job instead; mediaProcessing "converting media to different formats", video of the user starting it.
* [Keep the device awake](https://developer.android.com/develop/background-work/background-tasks/awake) — with an FGS and the screen off, if suspending would hurt the user "you might need to use a wake lock".

What an emulator cannot show and a phone can: CPU suspend with the screen off (the emulator never
suspends, so the missing wake lock is invisible here), OEM battery killers, thermal throttling of a
long CPU/GPU export, real Doze/App Standby on battery over hours, the full 6-hour budget and its reset,
Play Console review. Added to RESULT.md.

## 4. Defects found and fixed

Each defect, what was wrong, and the fix. Files changed: `RESULT.md`, `run_case.sh`,
`summarize.sh`, `queue.sh` (new; replaces `queue29.sh`, `queue31.sh`, `queue34.sh`, `queue37.sh`),
`app/fgslab/build.gradle.kts`, `app/fgslab/src/main/AndroidManifest.xml`,
`app/fgslab/src/{full,combo,none}/AndroidManifest.xml` (`src/noperm/` removed),
`app/fgslab/src/main/java/com/example/fgslab/{ExportService,MainActivity}.kt`,
`evidence/apk-md5.txt`, `evidence/a29-noneflavor-long/VOID-review-a1.txt` (new),
`evidence/review-a1-build-compare/` (new), the `evidence/review-a1-*` case directories, the APK sets
`apk-a1/`, `apk-a1-ts34/`, `apk-a1-ts33/` (new), and this file. `review-a1-queue.sh` drove the first
re-runs and was then folded into `queue.sh` (set `review-a1`) and removed.

**Evidence and harness**

* **D1 — false result: `a29-noneflavor-long`.** Claimed ✅(660); the case lasted 18 s and matched the previous case's `EXPORT_DONE` (section 1). Fix: void note in the case directory, RESULT.md row and procedure text corrected, case re-run as `review-a1-a29-noneflavor-long` (section 2).
* **D2 — `run_case.sh` could not tell its own run from the last one, and recorded too little.** It cleared only the default buffers, matched markers from any process (root cause of D1), recorded neither the APK md5 nor the device SDK (the mislabelled-emulator incident in RESULT.md's process note), took no `dumpsys` while the app was in the background (the brief asks for that evidence; `dumpsys_late.txt` is `(nothing)` after the export ends), and had no answer to an app crash: on 34/37 it waited out the full timeout when the process died at launch, and on 29 `am start -W` never returned once the app crashed behind its crash dialog (both seen in my first `bad-type-name` runs). Fix: `logcat -b all -c`; outcomes polled with `logcat --pid=<this run's pid>`; `apk_md5`, `device_sdk`, `pid` in `case.txt`; `dumpsys_bg.txt` + `resumed_bg.txt` 30 s after HOME; `PROCESS_GONE_AT_LAUNCH` when the process is gone at launch, `APP_CRASHED` on a `FATAL EXCEPTION` from the run's pid, and `am start -W` bounded to 30 s. Verified by the `review-a1-*` cases (the crash paths by `review-a1-fixed-a*-combo-bad-type-name`).
* **D3 — app: `onTimeout(int,int)` parameters misnamed** `(foregroundServiceType, timeoutSource)`; the platform signature is `onTimeout(int startId, int fgsType)`. RESULT.md quoted the resulting log as `fgsType=1 timeoutSource=2048`, i.e. it reported the startId as the type. Fix: `onTimeout(startId: Int, fgsType: Int)` logging `FGS_ON_TIMEOUT_2ARG startId=… fgsType=…` (verified: `review-a1-fixed-a37-combo-auto-knob60` logs `startId=1 fgsType=8192`); RESULT.md explains how to read the old builds' line.
* **D4 — APK provenance.** `evidence/apk-md5.txt` listed only `apk2/`, while the whole 29/31 matrix ran `apk/`, and RESULT.md called the `a29/a31-full-mediaproc` runs "byte-identical" to something they are not. Fix: md5s of all three builds recorded with which cases each ran; RESULT.md's Environment explains the builds (dex and manifest comparison); the "byte-identical" bullet rewritten.
* **D5 — RESULT.md was written (02:39) before the `a29/a31-combo-final` runs it reports ended (02:49).** The stated results held (checked), but the report asserted evidence that did not exist yet. Fix: recorded in the process note; all claims now checked against finished cases.
* **D6 — duplicated run scripts.** `queue29.sh`…`queue37.sh` were four near-identical copies (same APK variables and helper; 29/31 byte-identical but for serial and prefix), none checked the device SDK, and `summarize.sh` printed stale lines from other processes (it showed the bogus `EXPORT_DONE` for `a29-noneflavor-long`). Fix: one `queue.sh <serial> <api> <set> [apk-dir]` with sets `matrix` (the original per-level lists), `review-a1` and `smoke`, aborting when the device SDK differs; `summarize.sh` restricted to the case's own pid.

**RESULT.md content**

* **D7 — false: dataSync's 6 h/24 h budget "on 34 devices with target ≥ 34".** The limit is an Android 15 behaviour for apps targeting 35+ (ServiceInfo, timeout page) and the Android 14 device has no such mechanism (`review-a1-a34-combo-datasync-knob60`). Fix: corrected in the verdict, point 2 and the Timeouts table, with the new evidence.
* **D8 — false: "a declared-but-unused type costs nothing at runtime".** With the recommended manifest the type-less `startForeground(id, n)` throws on Android 14 (`review-a1-a34-combo-none`; on 37 it is accepted). Fix: verdict, API 34/37 tables, point 3 and the code comment now say the explicit type is mandatory.
* **D9 — overstated coverage.** "660 s backgrounded export per level" for dataSync (none on 37) and "no timeout on API 29" for shortService from a 45 s run. Fix: wording narrowed to what ran; shortService re-run past 180 s on 29 (`review-a1-a29-full-short-240`); 31 marked as documented only.
* **D10 — documentation citation errors.** The "caches the app" note attributed to the timeout page (it is on service-types/Android 15 pages), the specialUse sentence misquoted. Fix: attributions and quotes corrected; the Timeouts table rebuilt from the pages as read now.
* **D11 — recommended code was not the tested code and stopped short.** It showed `ServiceCompat.startForeground`, which the app never called, and no timeout handling; it advised implementing "both `onTimeout` overloads", though only `onTimeout(startId, fgsType)` applies to these types and the reference says both are not needed on 15+. Fix: snippet uses the tested `startForeground(id, n, type)`, adds `onTimeout(startId, fgsType)` → cancel + `stopSelf()` and stop-at-end, notes `ServiceCompat` as documented but not exercised; point 7 rewritten from the docs and the shortened-budget runs.
* **D12 — Play Console requirement missing, an unsupported claim in its place** ("a combined declaration dilutes Play review"). Fix: point 9 states the per-type declaration (description, user impact, video) for both declared types and Play's dataSync/mediaProcessing policy text.
* **D13 — emulator limits incomplete.** No mention that an FGS does not keep the CPU from suspending with the screen off (documented; invisible on an emulator), nor of thermal throttling lengthening a real export. Fix: point 10 (partial wake lock for the export's duration, to verify on a phone) and the emulator-limits list.

* **D14 — missing finding: the not-a-subset failure crashes the app.** In `a34/a37-combo-short-notdeclared` the caught `IllegalArgumentException` is followed ~70 ms later by `ForegroundServiceDidNotStartInTimeException` (`Bringing down service while still waiting for start foreground`); RESULT.md reported only the exception. The other failure kinds end without a crash. Fix: both table rows, a note under the API 34 table and point 3.

* **D15 — wrong expectation for the untested levels.** "Android 10/11/12 beta minor levels (30, 32, 33) — behavior should match 29/31/34 respectively" mislabels them (30 is Android 11, 32 is 12L, 33 is Android 13) and maps API 33 to API 34, although type and permission enforcement starts only with Android 14 ([changes to foreground services](https://developer.android.com/develop/background-work/services/fgs/changes)). Fix: the bullet names the versions and expects 29/31 behaviour, still marked not run.

**App code**

* **D16 — error handling.** An unknown `fgs_type` was resolved inside the `startForeground` try block, so a harness typo would have been logged as `STARTFOREGROUND_FAIL … IllegalArgumentException`, i.e. as a platform rejection; the export thread had no error handling, so an I/O failure would have killed it silently, left the FGS running with no work and the file open. Fix: type resolved before the try (a bad name now crashes: `review-a1-fixed-a34/a37-combo-bad-type-name`); `RandomAccessFile` in `use {}`, failures logged as `EXPORT_FAILED` and followed by `stopSelf()`.
* **D17 — duplicated code.** `TAG` defined in both classes, the 600 000 ms default in both, the `NotificationManager` lookup twice, and the `<service>` element copied into four flavor manifests; plus a dead `SDK_INT >= O` branch (minSdk 29). Fix: `MainActivity` uses `ExportService.TAG`/`DEFAULT_DURATION_MS`; one lazy `notificationManager`; the `<service>` is declared once in `src/main` with `android:foregroundServiceType="${fgsTypes}"`, each flavor sets `manifestPlaceholders["fgsTypes"]`, `none` removes the attribute with `tools:remove`, `full` adds only its `<property>`, `noperm` needs no manifest. The per-type `<uses-permission>` lines stay in the `full` and `combo` manifests: they *are* the per-variant fixture (which permissions each variant requests), and a product flavor cannot share a manifest with another flavor. Verified: `./gradlew assembleDebug` (+ `-PtargetSdk=34`, `-PtargetSdk=33`) builds without warnings; `aapt2 dump xmltree` of the four merged manifests equals `apk2/`'s attribute for attribute (full and none identical, combo and noperm differ only in element order; `evidence/review-a1-build-compare/`); `review-a1-fixed-*` runs on 29, 34 and 37.
* **D18 — the KDoc claimed the 10-minute default "exceed[s] all FGS timeouts we care about"**; it exceeds only shortService's 3 minutes. Fix: comment states what the default covers and how the 6-hour limits were reached (adb).

Checked and found correct, no change: notification channel (created before `startForeground`,
`IMPORTANCE_LOW`, notification `ONGOING_EVENT|ONLY_ALERT_ONCE|NO_CLEAR|FOREGROUND_SERVICE` in
`dumpsys`); permissions per flavor (merged manifests: `FOREGROUND_SERVICE` + `POST_NOTIFICATIONS`
everywhere, `DATA_SYNC`+`MEDIA_PROCESSING` in combo, + `SPECIAL_USE` in full, none in none/noperm);
`START_NOT_STICKY`; `stopSelf()` after a failed `startForeground` is the right lab behaviour (it
ends quietly after `Missing…`/`Invalid…`/`SecurityException`; after the not-a-subset failure the
platform crashes the app regardless, which is the D14 observation, not an app bug); both `onTimeout`
overrides call `stopSelf()` (no ANR or crash in any timeout case).

## 5. Agreement with the recommendation

**I agree with the recommendation** — `android:foregroundServiceType="mediaProcessing|dataSync"`,
`FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_MEDIA_PROCESSING` + `FOREGROUND_SERVICE_DATA_SYNC`, and
`mediaProcessing` when `SDK_INT >= 35`, else `dataSync` — and RESULT.md now carries it with three
amendments the evidence forces:

1. **The explicit type is mandatory.** With this manifest, Android 14 rejects the type-less
   `startForeground(id, notification)` (`review-a1-a34-combo-none`), and a type outside the manifest
   crashes the app even when the exception is caught (D14). The guide must show the three-argument
   call and nothing else.
2. **Timeouts are a 15+ concern only.** Neither type has a limit on Android 10–14 (documented, and
   shown on 34 with the adb budget). On 15+, for apps targeting 35+, mediaProcessing gets 6 hours of
   background time per 24 h; the guide must implement `onTimeout(startId, fgsType)` → cancel →
   `stopSelf()` (shown working on 37 with a 60 s budget) and stop the service when the export ends.
3. **Costs the build side did not state:** two Play Console declarations (dataSync and
   mediaProcessing, each with description, impact and video), and — documented, to be verified on a
   phone — a partial wake lock for the export's duration so it does not stall with the screen off.

Why agree: the same APK ran a 660 s backgrounded export on 29, 31, 34 and 37 (build side; re-run on
34 and 37 here, with in-background `dumpsys` showing `isForeground=true` while the launcher was
resumed); `mediaProcessing` is the type the documentation names for converting media and is
rejected on 14, so 35 is the right threshold; `dataSync` lists "Import or export operations" and
"Local file processing" and has no limit where it is used. The alternatives are worse for this job:
`shortService` stops at 180 s (re-run), `specialUse` needs a free-form justification that Play
reviews, an untyped service throws for any target ≥ 34, and WorkManager's long-running workers run
in a foreground service themselves (data-transfer guide), so they do not remove the type question.
Where I part from RESULT.md as handed over is in its supporting statements (section 4), not in the
choice.
