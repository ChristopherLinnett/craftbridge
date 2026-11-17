# CraftBridge issue #30 — Which foreground-service type does Android 10–14 accept for a long export?

**Investigation report, corrected by the review rounds A1, B1, A2, B2, … whose `REVIEW-<round>.md`
files sit next to it** (each lists the claims it checked, its fixes and its re-runs; the newest file is
the latest round). Rule for later rounds: each round
re-runs into new evidence directories (`CASE_TAG=<round>`) and leaves earlier evidence untouched; a re-run
that agrees with the earlier evidence apart from instance values (pids, timestamps, durations within the
run-to-run spread) is not a defect and does not change this report, which changes only where a claim, a
number or a citation is wrong, missing or unsupported. All runtime claims were produced by running the test app
(`app/`) on four local emulator AVDs; every claim cites a captured artifact under `evidence/<case>/`
(`case.txt`, `logcat_full.txt`, `dumpsys_early.txt`, `dumpsys_late.txt`, `install.txt`, `am_start.txt`;
the `review-a1-*` and `review-b1-*` cases also `dumpsys_bg.txt`, `resumed_bg.txt` and, for shortened
timeouts, `timeout_knob.txt`) or a quoted documentation passage with its URL.

## Verdict (one paragraph)

Declare **`android:foregroundServiceType="mediaProcessing|dataSync"`** in the manifest, request
`FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_MEDIA_PROCESSING` + `FOREGROUND_SERVICE_DATA_SYNC`, and
always pass the type explicitly at runtime: **`mediaProcessing` when `Build.VERSION.SDK_INT >= 35`,
else `dataSync`**. `mediaProcessing` (bit 0x2000) is an **Android 15 (API 35)** type; on the Android
14 platform it is **rejected** (`InvalidForegroundServiceTypeException: … type unknown … has been
prohibited`, cases `a34-full-mediaproc`, `a34-full-ts34-mediaproc`, `review-a1-a34-combo-mediaproc`),
and because the recommended manifest contains that bit, the **type-less** `startForeground(id,
notification)` is rejected the same way on Android 14 (`review-a1-a34-combo-none`) — so the explicit
type is mandatory, not a style choice. On Android 10/12 the unknown bit is accepted because those
platforms have no foreground-service-type **policy** at all — no unknown-bit check, no per-type
permission, no time limit — but they *do* still require the runtime type to be a subset of the declared
attribute: B1 re-ran the type-less and the not-declared cases on 29 and 31 and got `EXPORT_DONE` and
the identical `IllegalArgumentException … not a subset … 0x00002001` crash respectively
(`review-b1-a29-combo-none`, `review-b1-a29-combo-short-notdeclared`, `review-b1-a31-combo-none`,
`review-b1-a31-combo-short-notdeclared`). `dataSync` is accepted on every level measured — 29, 31, 34 and 37 (45 s runs on
all four; a 660 s backgrounded export on 29/31/34, where `auto` resolves to it); for 30, 32 and 33,
which were not run, Android's "Changes to foreground services" (`docs/fgs-changes.txt`) lists no change
that touches it: Android 11 adds only the camera and microphone types, 12L and 13 have no entry, and the
type and per-type-permission rules start with Android 14, so 30, 32 and 33 are expected to behave like 29
and 31, from the documentation, not measured. The single combo APK
(md5 `0cea776e2569a7e153abb28b42a2a29d`) completed a 660-second export started from the foreground and
sent to the background on **all four levels** (re-run in review A1 on 34 and 37). The reviewed rebuild
with the same manifest (`apk-a1/` = `apk-b1/`, md5 `3d45b867f8c088c64cd874303ccad7bc`; a different
file from `0cea…`, since review A1 changed the app code) ran 660 s on 34 and 37
(`review-b1-a34-combo-auto-long`, `review-b1-a37-combo-auto-long`) and on 29 and 31
(`review-a3-a29-combo-auto-long` `fileKiB=420224`, `review-a3-a31-combo-auto-long` `fileKiB=417792`). Neither chosen type
has any time limit on Android 10–14; on Android 15+ an app targeting 35+ gets 6 hours of background
time per type per 24 h, ending in `onTimeout(startId, fgsType)` — exercised here with the budget
shortened to 60 s by adb (`review-a1-a37-combo-auto-knob60`). `shortService` is unusable (180 s
platform timeout on 34 and 37); untyped services (no manifest attribute) throw on 34/37 for any app
targeting 34+; `specialUse` works on 29, 31, 34 and 37 (measured; 30, 32 and 33 expected from the
documentation) and is a fallback that needs Play justification and the
`PROPERTY_SPECIAL_USE_FGS_SUBTYPE` property. Both declared types must be declared in the Play
Console.

## Environment

| Item | Value |
|---|---|
| Host | Apple Silicon macOS, 18 cores, 64 GiB (two emulators at a time) |
| JDK (build) | Android Studio JBR 21.0.10 (`/Applications/Android Studio.app/Contents/jbr/Contents/Home`) |
| Gradle / AGP | Gradle 9.6.1 wrapper, AGP 9.5.1 (Kotlin via AGP built-in Kotlin support) |
| compileSdk / buildTools / minSdk / targetSdk | android-37.0 (`ro.build.version.sdk=37`) / 37.0.0 / 29 / 37 (plus ts34 and ts33 spot-check builds) |
| AVDs (`pixel_6` profile, `config.ini` checked in review A1) | `Pixel_6_API_29` ← `system-images;android-29;google_apis;arm64-v8a` · `Pixel_6_API_31` ← `system-images;android-31;google_apis;arm64-v8a` · `Pixel_6_API_34` ← `system-images;android-34;google_apis;arm64-v8a` · `Pixel_6_API_37` ← `system-images;android-37.2;google_apis_playstore_ps16k;arm64-v8a` (device reports `ro.build.version.sdk` = 37, fingerprint `…emu64a16k:17/CP41.260828.004.A7…:user`) |
| Builds measured (`ro.build.id` in each image's `build.prop`, security patch) | API 29 `QSR1.211112.011` (userdebug); API 31 `SE1A.220630.001.A1` (userdebug, 2022-08-01); API 34 `UE1A.230829.050` (userdebug, 2023-09-05), the first Android 14 release, the same one as the saved `android14-release` sources — Android 14 devices in the field run its QPR1–QPR3 builds, which were not checked, and `dataSync` below 35 is safe whether or not one of those knows `mediaProcessing`; API 37 `CP41.260828.004.A7` (2026-08-05). "Rejected on the Android 14 platform" below means on `UE1A.230829.050`. |
| Launch | `emulator -avd Pixel_6_API_NN -port 5554|5556 -no-window -no-audio -no-boot-anim -no-snapshot -gpu auto` |

Device SDKs confirmed at runtime: `adb shell getprop ro.build.version.sdk` → 29, 31, 34, 37 (the
`review-a1-*` and later cases record it in `case.txt` as `device_sdk=`; `target_sdk=` is recorded from
`review-b1-harness-smoke-*` and the `review-a2-*` cases on, the other `review-b1-*` cases ran the
earlier harness). End state is recorded at the end of each round's review file (§7 of the newest
`REVIEW-<round>.md`): emulators stopped, `com.example.fgslab` uninstalled, every `device_config` key written by
this work read back as `null`, flag sync `none` where the command exists (34, 37). `Pixel_6_API_29`
boot-looped after review B1 (`REVIEW-A2.md` §6); with the owner's approval it was wiped after review B2
and boots again (review A3 re-ran the API 29 cases on it).

APK sets and md5s: `evidence/apk-md5.txt`. Three builds ran:
* `apk/` (first build) ran the whole `a29-*`/`a31-*` matrix except `*-combo-final`. It differs from
  `apk2/` only by the `onTimeout(int)` override, which the platform never calls below API 34
  (`dexdump` of `ExportService`: `apk/` has `onTimeout(II)V` only, `apk2/` adds `onTimeout(I)V`); the
  binary manifests of the two combo APKs are byte-identical (`cmp`).
* `apk2/` (final build side) ran every `a34-*`/`a37-*` case, `a29/a31-combo-final` and all
  `review-a1-a*` re-runs. The pre-review `app/` source rebuilt it byte-for-byte (same four md5s), observed by the review-A1
reviewer before changing anything; no build log was kept and that source no longer exists.
* `apk-a1/` is `app/` after the review-A1 fixes; its merged manifests equal `apk2/`'s attribute for
  attribute (`aapt2 dump xmltree`; only the order of `<activity>`/`<service>` differs in combo and
  noperm), and the `review-a1-fixed-*` cases re-check it on the devices.
* `apk-b1/` (+ `apk-b1-ts34/`, `apk-b1-ts33/`) is review B1's clean rebuild of the **unchanged** `app/`
  source. All four APK md5s are identical to `apk-a1/` (combo `3d45b867f8c088c64cd874303ccad7bc`, full
  `8e76be90bf0ea2a6aaee830dd0873506`, none `16fad23e7bf3978c928a81eaac0df26b`, noperm
  `15b2745f2b375a3a020f5e40707f7612`) — proof that the packaging is reproducible and that the source now
  in `app/` is exactly what review A1 tested. Every `review-b1-*` case ran on these files;
  `assembleDebug`, `-PtargetSdk=34` and `-PtargetSdk=33` all ended `BUILD SUCCESSFUL` with zero warning
  lines (`logs/b1-build-*.log`).
* Which build each later round ran (`apk_md5=` in every review round's `case.txt`; the 49 build-side
  cases predate that field — their `apk/`/`apk2/` sets are named above): every `review-<round>-*`
  case from review B1 on ran `apk-b1/`, except the `review-b2-*` cases outside `review-b2-b1apk-*`,
  which ran `apk2/` because `queue.sh` then defaulted to that directory. This report cites no
  `review-b2-*` case.

### Test app (`app/`)

Kotlin, no external dependencies. `MainActivity` (started via `adb shell am start` = the simulated
user tap) starts `ExportService` with `startForegroundService()` and extras `fgs_type`
(`none|dataSync|mediaProcessing|specialUse|shortService|auto`) and `fgs_duration_ms`. `auto` =
`mediaProcessing` on SDK≥35 else `dataSync` — i.e. the recommendation itself under test.
`ExportService.startExport()` calls `startForeground(id, notif)` (type `none`) or
`startForeground(id, notif, typeBit)` inside a try/catch that logs the exact exception, then runs a
CPU-bound loop (≈40 ms of `sqrt` math + a 64 KiB append per tick, then a 60 ms sleep) to
`filesDir/export.bin`, at **≈630 KiB/s sustained** — `TICK elapsedS=60 fileKiB=38208` in the first
minute, 417,280 KiB written in a 660 s run, i.e. ~632 KiB/s average — posting progress to the
notification every 5 s and a `TICK` log line every 60 s. `onTimeout(startId)` (API 34, shortService) and
`onTimeout(startId, fgsType)` (API 35) are both overridden and logged, and call `stopSelf()`. Log marker
`CraftFGS`.

Note on the logs: in the `apk/`/`apk2/` builds the two-argument override named its parameters
`(foregroundServiceType, timeoutSource)`, but the platform signature is
`onTimeout(int startId, int fgsType)`. Their log line `FGS_ON_TIMEOUT fgsType=A timeoutSource=B`
therefore means **startId=A, fgsType=B**. The `apk-a1/` build logs
`FGS_ON_TIMEOUT_2ARG startId=… fgsType=…`.

Gradle flavors isolate the manifest question. The `<service>` is declared once in `src/main`; each
flavor sets `android:foregroundServiceType` through the `fgsTypes` manifest placeholder and adds
only its own permissions:
* `full` — `dataSync|mediaProcessing|specialUse|shortService` + all per-type permissions + the specialUse `<property>` → tests what installs and what the runtime accepts per type.
* `combo` — `mediaProcessing|dataSync` (+ their permissions) → **the recommendation**.
* `none` — no `foregroundServiceType` attribute at all (`tools:remove`).
* `noperm` — `dataSync|mediaProcessing|specialUse` but **no** per-type permission → captures the permission-enforcement exceptions.

Build: `cd app && ./gradlew assembleDebug` (verified in review A1; also `-PtargetSdk=34` /
`-PtargetSdk=33` for the target-SDK spot checks).

### Test procedure (every case, via `run_case.sh`; device sets via `queue.sh`)

`adb logcat -b all -c` → `adb install -r -g <apk>` → `adb shell am start -W -n com.example.fgslab/.MainActivity --es fgs_type <t> --el fgs_duration_ms <ms>` → 8 s → `dumpsys activity services com.example.fgslab` → device time (`home_sent_device=`, from review A2 on) and `adb shell input keyevent KEYCODE_HOME` (app to background) → 30 s later `dumpsys activity services` + resumed activity + `dumpsys activity processes` with the device time (`dumpsys_bg.txt`, `resumed_bg.txt`, `processes_bg.txt`, the last from review A2 on) → poll this run's pid (`logcat --pid`) until `EXPORT_DONE` / `STARTFOREGROUND_FAIL` / `FGS_ON_TIMEOUT` / `FATAL EXCEPTION` / process gone (`am start -W` is bounded to 30 s because it never returns on API 29 after a crash) → capture `logcat -d -v time` + a last `dumpsys`, and append `target_sdk=` and, after a crash of the app, `fatal_exception=` / `fatal_exception_at_launch=` (the exception and `Caused by:` class names of that crash). Every file the script writes is removed at case start, so re-using a label cannot mix two runs, and crash fields come only from the run's own pid (on API 29 the clear leaves the previous case's crash in the buffer). `queue.sh` requires the APK directory. Timeout knobs (`queue.sh`) suspend `device_config` server sync while set, because a Play-services flag sync dropped a local override mid-run on both 34 and 37 (review A2). Started from the foreground in every case, per the task.

The build-side cases ran an earlier `run_case.sh` that cleared only the default log buffers, matched
outcome lines from any process and took no in-background `dumpsys`. On API 29 the clear left the
previous case's `EXPORT_DONE` in the buffer, so `a29-noneflavor-long` "finished" after 18 s on a
stale line; it was re-run as `review-a1-a29-noneflavor-long`. (`review-a1-a29-noneflavor-long-aborted`
was ended at 535 s by the reviewer's `adb uninstall`, not by Android; see REVIEW-A1.md.) Every other build-side case was checked
to end on a marker from its own process (review A1).

## Results

Legend: ✅ = `startForeground` accepted and the export **ran to completion after HOME**; ✅(45) = 45 s run; ✅(660) = 660 s (11 min) endurance run; ❌ = `startForeground` threw (exact text quoted). "Manifest" column = flavor's declared `foregroundServiceType`.

### API 29 (Android 10, Pixel_6_API_29) — no foreground-service-type *policy*, but the manifest subset rule already applies

| Type passed | Manifest (full = all four) | Result | Evidence |
|---|---|---|---|
| none (no arg) | all four | ✅(45) `STARTFOREGROUND_OK requested=none typeBit=0` | `a29-full-none` |
| none (no arg) | none flavor (attribute absent) | ✅(45); ✅(660) `STARTFOREGROUND_OK requested=none typeBit=0` → `EXPORT_DONE type=none elapsedS=660 fileKiB=421504`, bg dumpsys `isForeground=true` with the launcher resumed | `a29-noneflavor-none`, `review-a1-a29-noneflavor-long` (`a29-noneflavor-long` is void: stale log line, see above) |
| dataSync | all four | ✅(45) | `a29-full-datasync` |
| **mediaProcessing** | all four | ✅(45) `STARTFOREGROUND_OK requested=mediaProcessing typeBit=8192` — **unknown bit accepted, install succeeds** | `a29-full-mediaproc` |
| specialUse | all four | ✅(45) typeBit=1073741824 | `a29-full-specialuse` |
| shortService | all four | ✅(45) typeBit=2048; **no timeout on API 29**: a 240 s run passed the 180 s mark (`TICK elapsedS=180`) and ended `EXPORT_DONE type=shortService elapsedS=240`, no `onTimeout`, no ANR | `a29-full-short`, `review-a1-a29-full-short-240` |
| dataSync / mediaProcessing | noperm flavor (no per-type permission) | ✅(45) — permission checks not present on API 29 | `a29-noperm-datasync`, `a29-noperm-mediaproc` |
| auto (=dataSync) | combo | ✅(660) `STARTFOREGROUND_OK requested=auto typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=415232`; final-APK run `a29-combo-final` `EXPORT_DONE elapsedS=660 fileKiB=417280` | `a29-combo-auto-long`, `a29-combo-final` |
| none (no arg) | combo (`mediaProcessing|dataSync`) | ✅(45) `STARTFOREGROUND_OK requested=none typeBit=0` → `EXPORT_DONE` — with no type policy the unknown manifest bit costs nothing here | `review-b1-a29-combo-none` |
| **shortService** | combo (not declared) | ❌ `java.lang.IllegalArgumentException: foregroundServiceType 0x00000800 is not a subset of foregroundServiceType attribute 0x00002001 in service element of manifest file` (caught) → the process then dies with `android.app.RemoteServiceException: Context.startForegroundService() did not then call Service.startForeground()` | `review-b1-a29-combo-short-notdeclared` |
| mediaProcessing | combo | ✅(45) `typeBit=8192` (B1 re-check of the review-A1 run, same conclusion) | `review-b1-a29-combo-mediaproc` |

What API 29 does not have is the **type policy**: `ForegroundServiceTypePolicy` (the class that maps a
type to its required permissions, deprecation and disabled state — see the API 34 section) simply did not
exist, so an unknown type bit, a missing `FOREGROUND_SERVICE_DATA_SYNC`-style permission and any time
limit are all ignored. What it *does* still enforce is that the type passed to `startForeground()` is a
**subset of the manifest attribute** — the identical message as on 34/37, and the same fatal outcome
when the caught exception leaves `startForegroundService()` without a `startForeground()` call. Review B1
re-ran exactly these two cases on the recommendation's own manifest to pin that down
(`review-b1-a29-combo-none`, `review-b1-a29-combo-short-notdeclared`), and the same pair on API 31
(`review-b1-a31-combo-none`, `review-b1-a31-combo-short-notdeclared`).

Install of the manifest that declares `mediaProcessing`/`specialUse`/`shortService` bits, the
`<property>` element and the unknown `FOREGROUND_SERVICE_*` permissions: `install_rc=0` and
`Success` in every `install.txt` of all 49 build-side cases on all four levels. So *declaring*
platform-unknown values never blocks install (the compiled attribute is an integer of flag bits).

### API 31 (Android 12, Pixel_6_API_31) — same story: no type policy, same manifest subset rule

Identical matrix to API 29: every scenario ✅ including `mediaProcessing` (typeBit=8192, `a31-full-mediaproc`), `specialUse` (`a31-full-specialuse`), `shortService` for 45 s (`a31-full-short`; no 3-minute run on 31 — the documented behaviour for Android 13 and lower is "doesn't receive a timeout, nor does it ANR", see Timeouts), noperm variants ✅ (`a31-noperm-*`), untyped ✅(45)/(660) (`a31-noneflavor-none`, `a31-noneflavor-long` `EXPORT_DONE type=none elapsedS=660`), combo auto=dataSync ✅(660) (`a31-combo-auto-long`, final-APK run `a31-combo-final`). Review B1 re-ran the recommendation's own manifest on 31 (`review-b1-a31-combo-auto`, `review-b1-a31-combo-none`, `review-b1-a31-combo-mediaproc` — all `EXPORT_DONE`, so runtime `mediaProcessing` **is** accepted on API 31, where there is no type policy) and the not-declared case (`review-b1-a31-combo-short-notdeclared`: same `IllegalArgumentException … not a subset … 0x00002001`, then the fatal `android.app.ForegroundServiceDidNotStartInTimeException`).

### API 34 (Android 14, Pixel_6_API_34) — enforcement switches on (targetSdk ≥ 34)

| Type passed | Manifest | Result | Evidence |
|---|---|---|---|
| none (no arg) | full (`dataSync|mediaProcessing|specialUse|shortService`) | ✅(45); `dumpsys` shows `types=40002001` — the manifest types incl. the unknown 0x2000 bit, shortService dropped (`startForeground(): FOREGROUND_SERVICE_TYPE_SHORT_SERVICE is combined with other types. SHORT_SERVICE will be ignored.`) | `a34-full-none` |
| **none (no arg)** | **combo (`mediaProcessing|dataSync`)** | ❌ `android.app.InvalidForegroundServiceTypeException: Starting FGS with type unknown callerApp=ProcessRecord{…} targetSDK=37 has been prohibited` — the type-less call takes the manifest types, and the unknown `mediaProcessing` bit is rejected | `review-a1-a34-combo-none`, `review-b1-a34-combo-none` |
| none (no arg) | none flavor | ❌ `android.app.MissingForegroundServiceTypeException: Starting FGS without a type callerApp=ProcessRecord{…} targetSDK=37` (same with `targetSDK=34` for ts34 build) | `a34-noneflavor-none`, `a34-none-ts34-none` |
| none (no arg) | none flavor, **targetSdk 33 build** | ✅(45) — enforcement is target-SDK gated: `SERVICE_CREATED sdk=34 targetSdk=33` → `STARTFOREGROUND_OK` | `a34-none-ts33-none` |
| dataSync | full / combo | ✅(45); combo auto ✅(660) `typeBit=1 → EXPORT_DONE elapsedS=660` (review re-run: `09:26:53.725 STARTFOREGROUND_OK requested=auto typeBit=1` → `09:37:53.824 EXPORT_DONE elapsedS=660 fileKiB=416512`; `dumpsys_bg.txt` `isForeground=true … types=00000001` while the launcher is resumed) | `a34-full-datasync`, `review-a1-a34-combo-datasync`, `review-b1-a34-combo-datasync`, `a34-combo-auto-long`, `review-a1-a34-combo-auto-long`, `review-b1-a34-combo-auto-long` |
| dataSync, timeout budget set to 60 s by adb | combo | ✅ ran its full 150 s in the background: Android 14 has **no** dataSync time limit (`am compat enable FGS_INTRODUCE_TIME_LIMITS` → `Unknown or invalid change`; the change id is absent from `dumpsys platform_compat`, and the key itself does not exist in Android 14's `ActivityManagerConstants`). The runs that show the key in force for the whole run are `review-b1-a34-combo-datasync-knob60` (deleted successfully afterwards) and `review-a2-guarded-a34-combo-datasync-knob60` (config sync suspended, read back `60000` before deletion); in `review-a1-` and `review-a2-a34-combo-datasync-knob60` a server flag sync removed the key mid-run (`Failed to delete` afterwards), so those two prove nothing on their own | `review-a1-a34-combo-datasync-knob60`, `review-b1-a34-combo-datasync-knob60`, `review-a2-guarded-a34-combo-datasync-knob60` |
| **mediaProcessing** | full / combo (declared) | ❌ `android.app.InvalidForegroundServiceTypeException: Starting FGS with type unknown callerApp=ProcessRecord{…} targetSDK=37 has been prohibited` — **also fails with targetSdk 34 build** | `a34-full-mediaproc`, `a34-full-ts34-mediaproc`, `review-a1-a34-combo-mediaproc`, `review-b1-a34-combo-mediaproc` |
| specialUse | full (+`<property>` subtype) | ✅(45) and ✅(660) `EXPORT_DONE type=specialUse elapsedS=660` | `a34-full-specialuse`, `a34-full-specialuse-long` |
| shortService | full | started OK (`typeBit=2048`), **platform timed it out after 180.0 s**: `01:54:59.821 STARTFOREGROUND_OK` → `01:57:59.828 ActivityManager: Short FGS timed out` → `01:57:59.829 FGS_ON_TIMEOUT_1ARG startId=1` (API-34 callback; the 2-arg overload does not exist on 34) → `EXPORT_ABORTED elapsedS=180`; re-run `09:20:44.751` → `09:23:44.755`, again 180.0 s, and a third time in B1 `10:40:54.212` → `10:43:54.229` (`review-b1-a34-full-short-timeout`) | `a34-full-short-timeout`, `review-a1-a34-full-short-timeout`, `review-b1-a34-full-short-timeout` |
| shortService | combo (not declared) | ❌ `java.lang.IllegalArgumentException: foregroundServiceType 0x00000800 is not a subset of foregroundServiceType attribute 0x00002001 in service element of manifest file` — and although the app catches it and calls `stopSelf()`, the platform then kills the app: `ActivityManager: Bringing down service while still waiting for start foreground` → `FATAL EXCEPTION … RemoteServiceException$ForegroundServiceDidNotStartInTimeException: Context.startForegroundService() did not then call Service.startForeground()` | `a34-combo-short-notdeclared`, `review-b1-a34-combo-short-notdeclared` |
| dataSync | noperm | ❌ `java.lang.SecurityException: Starting FGS with type dataSync … targetSDK=37 requires permissions: all of the permissions allOf=true [android.permission.FOREGROUND_SERVICE_DATA_SYNC]` | `a34-noperm-datasync` |
| mediaProcessing | noperm | ❌ `InvalidForegroundServiceTypeException: … type unknown … prohibited` — the unknown-type policy code is returned before the missing permission is ever consulted (see the source note below the table) | `a34-noperm-mediaproc` |
| specialUse | noperm | ❌ `java.lang.SecurityException: … type specialUse … requires permissions: all of the permissions allOf=true [android.permission.FOREGROUND_SERVICE_SPECIAL_USE]` | `a34-noperm-specialuse` |

Of all the `startForeground` failures, only the not-a-subset `IllegalArgumentException` is followed
by that crash; after the `Missing…`, `Invalid…` and `SecurityException` failures the same
catch-and-`stopSelf()` path ends quietly (no `FATAL EXCEPTION` anywhere in the
`a34-noneflavor-none`, `a34/a37-noperm-*` or `review-b1-a34-combo-none` logs; `a34-full-none` and
`review-b1-a37-combo-none` are accepted starts, not failures). So catching the exception is no safety net: the type passed must be one the manifest declares.
The fatal exception is spelled differently per platform, which is worth one line in the guide — same
cause, the process dies within a second of the caught exception on every level (55–192 ms in eight
runs on 29/31/34/37, 649 ms in `review-a2-a31-combo-short-notdeclared`: the system tears the service down at
once, `Bringing down service while still waiting for start foreground`, and the crash is thrown when the
app's main thread next runs), three class names:
`android.app.RemoteServiceException:
Context.startForegroundService() did not then call Service.startForeground()` on API 29
(`review-b1-a29-combo-short-notdeclared`), `android.app.ForegroundServiceDidNotStartInTimeException` on
API 31 (`review-b1-a31-combo-short-notdeclared`), and
`android.app.RemoteServiceException$ForegroundServiceDidNotStartInTimeException` on 34 and 37
(`review-b1-a34-combo-short-notdeclared`, `a37-combo-short-notdeclared`).

The type-less rows show an asymmetry no developer documentation mentions: with a multi-type manifest
value Android 14 **accepts** the unknown `mediaProcessing` bit when `specialUse` is declared next to it
(`a34-full-none`) and **rejects** it when it is not (`review-a1-a34-combo-none`, re-run in B1 as
`review-b1-a34-combo-none`, byte-identical build, identical message). The Android 14 page only says the
call then defaults to the manifest types and must satisfy **all** of them. The platform source does
state the rule, and it is exactly what those two runs show: `ActiveServices` splits the effective type
bit by bit and chooses a fallback policy for bits the platform does not know —

```java
// If the service has declared some unknown types which might be coming
// from future releases, and if it also comes with the "specialUse",
// then it'll be deemed as the "specialUse" and we ignore this
// unknown type. Otherwise, it'll be treated as an invalid type.
int defaultFgsTypes = (foregroundServiceType
        & ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE) != 0
        ? ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        : ServiceInfo.FOREGROUND_SERVICE_TYPE_NONE;
```

([`ActiveServices.java`, android14-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/services/core/java/com/android/server/am/ActiveServices.java),
`setServiceForegroundInnerLocked`, local copy `docs/aosp-android14-release-ActiveServices.java`).
Every single bit is then looked up in `ForegroundServiceTypePolicy`'s table of types the platform knows
([`ForegroundServiceTypePolicy.java`, android14-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/core/java/android/app/ForegroundServiceTypePolicy.java),
local copy `docs/aosp-android14-release-ForegroundServiceTypePolicy.java`): `mForegroundServiceTypePolicies.get(type)`,
and on a miss "Unknown type, fallback to the defaultToType". With `specialUse` in the value the unknown
bit is therefore judged as `specialUse` (permission `FOREGROUND_SERVICE_SPECIAL_USE`, which the `full`
flavor has) → accepted. Without it the fallback is the `NONE` policy, which for targetSdk ≥ 34 is
disabled → `FGS_TYPE_POLICY_CHECK_DISABLED` → `InvalidForegroundServiceTypeException … has been
prohibited` in `validateForegroundServiceType`. The same code path explains `a34-noperm-mediaproc`: each
bit yields exactly one policy code, and for an unknown bit the code is `FGS_TYPE_POLICY_CHECK_DISABLED`,
so the missing `FOREGROUND_SERVICE_MEDIA_PROCESSING` permission is never what the call fails on —
measured: on 14 the noperm build throws the *type* exception, on 37 (where the type exists) it throws
`SecurityException` (`a34-noperm-mediaproc` vs `a37-noperm-mediaproc`).

Two consequences for the guide. (a) On Android 14 a type-less `startForeground()` survives an unknown
future type bit in the manifest only next to `specialUse` — which is not the manifest recommended here —
while an explicit known type is unaffected by it (every `auto` = dataSync run on 34) — so passing the type
explicitly is the supported path, not a style preference. (b) The fallback above is platform-source
behaviour with no counterpart in the public documentation (and in the same constructor the platform
comments that it `will default to "none"` for the hidden `fileManagement` type, i.e. this fallback is
where undocumented type values land), so nothing in a guide should lean on it.

### API 37 (Android 17 image, Pixel_6_API_37) — mediaProcessing is first-class

| Type passed | Manifest | Result | Evidence |
|---|---|---|---|
| none (no arg) | full | ✅(45), `types=0x40002001` | `a37-full-none` |
| none (no arg) | combo | ✅(45) | `review-a1-a37-combo-none`, `review-b1-a37-combo-none` |
| none (no arg) | none flavor | ❌ `MissingForegroundServiceTypeException: Starting FGS without a type … targetSDK=37` | `a37-noneflavor-none` |
| dataSync | full / combo | ✅(45) (no 660 s dataSync run on 37: there `auto` resolves to mediaProcessing) | `a37-full-datasync`, `review-a1-a37-combo-datasync`, `review-b1-a37-combo-datasync` |
| dataSync, budget set to 60 s by adb | combo | **timed out**: HOME `09:25:49`, `09:26:50.332 ActivityManager: FGS (dataSync) timed out` → `FGS_ON_TIMEOUT fgsType=1 timeoutSource=1` (= startId 1, fgsType 1 = dataSync) → `stopSelf()` → `EXPORT_ABORTED elapsedS=69`; no crash | `review-a1-a37-combo-datasync-knob60`, `review-b1-a37-combo-datasync-knob60` |
| **mediaProcessing** | full / combo | ✅(45) `STARTFOREGROUND_OK requested=mediaProcessing typeBit=8192`; **ts34 build also ✅ on API 37** (the type exists on the device regardless of targetSdk) | `a37-full-mediaproc`, `review-a1-a37-combo-mediaproc`, `a37-full-ts34-mediaproc`, `review-b1-a37-combo-mediaproc` |
| specialUse | full (+`<property>`) | ✅(45) and ✅(660) | `a37-full-specialuse`, `a37-full-specialuse-long` |
| shortService | full | started `01:55:28.103`, **both callbacks fired at 180.0 s**: `01:58:28.109 FGS_ON_TIMEOUT_1ARG startId=1` + `FGS_ON_TIMEOUT fgsType=1 timeoutSource=2048` (= startId 1, fgsType 2048 = shortService) → `EXPORT_ABORTED elapsedS=180` (our `stopSelf()` inside the grace period; no ANR in the log) | `a37-full-short-timeout` |
| shortService | combo (not declared) | ❌ same `IllegalArgumentException … not a subset … 0x00002001`, then the same `ForegroundServiceDidNotStartInTimeException` crash | `a37-combo-short-notdeclared`, `review-b1-a37-combo-short-notdeclared` |
| dataSync | noperm | ❌ `SecurityException: … type dataSync … requires permissions: all of the permissions [android.permission.FOREGROUND_SERVICE_DATA_SYNC]` | `a37-noperm-datasync` |
| mediaProcessing | noperm | ❌ `SecurityException: … type mediaProcessing … requires permissions: all of the permissions [android.permission.FOREGROUND_SERVICE_MEDIA_PROCESSING]` | `a37-noperm-mediaproc` |
| **auto (=mediaProcessing on 35+)** | combo | ✅(660) `STARTFOREGROUND_OK requested=auto typeBit=8192` → `EXPORT_DONE elapsedS=660 fileKiB=408256`; review re-run `09:27:20.276 STARTFOREGROUND_OK requested=auto typeBit=8192` → `09:38:20.286 EXPORT_DONE elapsedS=660 fileKiB=409664`, `dumpsys_bg.txt` `types=0x00002000` with the launcher resumed | `a37-combo-auto-long`, `review-a1-a37-combo-auto-long`, `review-b1-a37-combo-auto-long` (B1 `fileKiB=413440`) |
| **auto (=mediaProcessing)**, budget set to 60 s by adb | combo | **timed out**: `STARTFOREGROUND_OK 09:24:01.849`, HOME `09:24:10.0`, `09:25:11.030 ActivityManager: FGS (mediaProcessing) timed out` → `FGS_ON_TIMEOUT fgsType=1 timeoutSource=8192` (= startId 1, fgsType 8192 = mediaProcessing) → `stopSelf()` → `EXPORT_ABORTED elapsedS=69`; no crash. The 60 s ran out 61 s after HOME, 69 s after `startForeground` | `review-a1-a37-combo-auto-knob60`, `review-b1-a37-combo-auto-knob60` |

`dumpsys activity services com.example.fgslab` shows the FGS state per case, e.g. (combo auto on 37):
`isForeground=true foregroundId=4711 types=0x00002000 foregroundNoti=Notification(channel=export … flags=ONGOING_EVENT|ONLY_ALERT_ONCE|NO_CLEAR|FOREGROUND_SERVICE …)`
(`dumpsys_early.txt`, taken 8 s after the start; in the `review-a1-*` cases also `dumpsys_bg.txt`,
taken 30 s after HOME while `resumed_bg.txt` shows the launcher as the resumed activity).
`dumpsys_late.txt` is taken after the export ended and is normally `(nothing)`.

### Review-B1 re-runs (source unchanged, APKs byte-identical to `apk-a1/`)

Review B1 rebuilt `app/` from scratch (`apk-b1*`, md5s identical to `apk-a1*`), re-ran the amendment
cases on every level and cross-checked every other claim against the evidence; `logs/b1-verify.txt`
lists, per case, the APK md5, the device SDK, the target SDK, the pid and the outcome markers. Harness
changes in this round: `queue.sh` takes `CASE_TAG` so a re-run writes new evidence instead of
overwriting the previous round's, and it now reads every `device_config` key back after deleting it.

| Case (`evidence/…`) | Device | Result |
|---|---|---|
| `review-b1-a34-combo-none` | 34 | ❌ `InvalidForegroundServiceTypeException: … type unknown … targetSDK=37 has been prohibited` (identical to A1) |
| `review-b1-a34-combo-mediaproc` | 34 | ❌ same exception, explicit type |
| `review-b1-a34-combo-datasync` | 34 | ✅ `typeBit=1` → `EXPORT_DONE`; bg dumpsys `types=00000001` |
| `review-b1-a34-combo-short-notdeclared` | 34 | ❌ not-a-subset → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` |
| `review-b1-a34-full-short-timeout` | 34 | `FGS_ON_TIMEOUT_1ARG` at **180.0 s**, `EXPORT_ABORTED elapsedS=180`, no ANR |
| `review-b1-a34-combo-datasync-knob60` | 34 | ✅ ran its full 150 s: the 60 s budget is ignored on 14; `get`→`60000`, `delete`→ read back `null`; `dumpsys platform_compat` has no `FGS_INTRODUCE_TIME_LIMITS` |
| `review-b1-a34-combo-auto-long` | 34 | ✅ `typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=419392`, bg dumpsys `types=00000001` |
| `review-b1-a37-combo-none` | 37 | ✅ **accepted** on 37 (`types=0x00002001`) → `EXPORT_DONE` |
| `review-b1-a37-combo-mediaproc` / `-datasync` | 37 | ✅ ✅ |
| `review-b1-a37-combo-short-notdeclared` | 37 | ❌ not-a-subset → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` |
| `review-b1-a37-combo-auto-knob60` | 37 | `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192` 61.07 s after HOME, `EXPORT_ABORTED elapsedS=69`, `stopSelf()` → no crash |
| `review-b1-a37-combo-datasync-knob60` | 37 | `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=1` 61.08 s after HOME, `elapsedS=69` |
| `review-b1-a37-combo-auto-long` | 37 | ✅ `typeBit=8192` → `EXPORT_DONE elapsedS=660 fileKiB=413440`, bg dumpsys `types=0x00002000` |
| `review-b1-a37-exhaust-1st-timeout` / `-2nd-foreground` | 37 | budget exhausted → timeout; a **foreground** start of the same type right after is accepted and completes (knob file in `review-b1-a37-datasync-exhaust-reset/`) |
| `review-b1-a31-combo-auto` / `-combo-mediaproc` / `-combo-none` | 31 | ✅ ✅ ✅ (no type policy on 12) |
| `review-b1-a31-combo-short-notdeclared` | 31 | ❌ not-a-subset → `android.app.ForegroundServiceDidNotStartInTimeException` |
| `review-b1-a31-combo-bad-type-name` | 31 | the app's own `IllegalArgumentException: unknown type name: bogus`, uncaught in `onStartCommand` → crash (by design: an unknown name is a programming error) |
| `review-b1-a29-combo-none` / `-combo-mediaproc` | 29 | ✅ ✅ |
| `review-b1-a29-combo-short-notdeclared` | 29 | ❌ not-a-subset → `android.app.RemoteServiceException` |

Every B1 case except that labelled void logged `Success` for the install, and the `device_sdk=` in each
`case.txt` matches its `a<api>-` label. Two further directories are harness self-checks of the changed `run_case.sh`
(`review-b1-harness-smoke-*`) and one is a labelled void: `review-b1-a29-noperm-datasync` never reached
the platform (`adb install` → `Can't find service: package`, that emulator was mid-shutdown), so it
supports no claim and its `VOID-review-b1.txt` says so. The noperm permission evidence did not need
re-running: the `noperm` flavour is unchanged and its APK md5 is identical to the one that produced the
`SecurityException` cases (`a34/a37-noperm-*`), and `aapt2 dump xmltree` of that APK shows
`FOREGROUND_SERVICE` + `POST_NOTIFICATIONS` only, with `foregroundServiceType=0x40002001`.

### One-APK-on-all-levels proof

Same file `apk2/fgslab-combo-ts37.apk`, md5 `0cea776e2569a7e153abb28b42a2a29d`,
manifest `android:foregroundServiceType="mediaProcessing|dataSync"`, type chosen at runtime:

| Device | Runtime type chosen | startForeground | 660 s export | Evidence |
|---|---|---|---|---|
| API 29 | dataSync (bit 1) | OK | `EXPORT_DONE elapsedS=660 fileKiB=417280`; A3 re-run with the `apk-b1/` combo (`3d45…`, same manifest) `fileKiB=420224` | `a29-combo-final`, `review-a3-a29-combo-auto-long` |
| API 31 | dataSync (bit 1) | OK | `EXPORT_DONE elapsedS=660 fileKiB=414528`; A3 re-run with the `apk-b1/` combo `fileKiB=417792` | `a31-combo-final`, `review-a3-a31-combo-auto-long` |
| API 34 | dataSync (bit 1) | OK | `EXPORT_DONE elapsedS=660 fileKiB=413632`; re-run `fileKiB=416512`; B1 re-run with the `apk-b1/` combo (`3d45…`, same manifest) `fileKiB=419392` | `a34-combo-auto-long`, `review-a1-a34-combo-auto-long`, `review-b1-a34-combo-auto-long` |
| API 37 | mediaProcessing (bit 8192) | OK | `EXPORT_DONE elapsedS=660 fileKiB=408256`; re-run `fileKiB=409664`; B1 re-run with the `apk-b1/` combo (`3d45…`, same manifest) `fileKiB=413440` | `a37-combo-auto-long`, `review-a1-a37-combo-auto-long`, `review-b1-a37-combo-auto-long` |

(`a29-combo-auto-long`/`a31-combo-auto-long` ran the first build `apk/`, see Environment; the
`apk2/` APK was then re-run on 29/31 as `a29-combo-final`/`a31-combo-final` — both logs assert
`SERVICE_CREATED sdk=29`/`sdk=31`, and every case directory in `evidence/` was cross-checked so its
log's device SDK matches its `a<api>-` label.) The review-A1 rebuild `apk-a1/` was checked on 29,
34 and 37 with short runs (`review-a1-fixed-a*`): `auto` ran 45 s on all three (`typeBit=1` on 29
and 34, `typeBit=8192` on 37); explicit `mediaProcessing` was accepted on 29 and 37 and rejected on 34
with the same `InvalidForegroundServiceTypeException`; a 60 s mediaProcessing budget on 37 ended in
`FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192`; an unknown `fgs_type` now crashes the app
(`unknown type name: bogus`) instead of being logged as a platform rejection. Review B1 re-ran those
same bytes on **all four** levels — 29, 31, 34 and 37 — which is also what closes the API 31 gap (the
review-A1 set had not exercised `apk-a1` there).

### Process note (kept for transparency)

An earlier attempt at the `a29/a31-combo-final` re-runs silently targeted **stale emulator processes**:
the API 34/37 pair was left running on ports 5554/5556 when the replacement 29/31 emulators were
started, so the commands hit the old devices (their logs showed `SERVICE_CREATED sdk=34` / `sdk=37`).
Those mislabeled evidence directories were deleted, all emulators were stopped, the ports were
re-verified with `adb shell getprop ro.build.version.sdk` before any re-run, and the full evidence set
was cross-checked label-vs-device-SDK after the fact (result: all correct). No conclusion in this
report rests on the discarded runs. `queue.sh` now refuses to run when the device's SDK differs from
the level asked for. The first version of this report (02:39) was written while the
`a29/a31-combo-final` runs were still going (they ended 02:49); review A1 checked that their logs
show the results the report had already stated.

## Timeouts — observed vs documented

| Type / device | Observed on emulator | Documented |
|---|---|---|
| shortService, device 34 | timed out at **180.0 s** (three runs: 01:57:59.829, 09:23:44.755, and 10:43:54.229 on the B1 build, `review-b1-a34-full-short-timeout`) via `onTimeout(int)`; our prompt `stopSelf()` avoided the ANR | "Can only run for a short period of time (about 3 minutes)" ([service-types](https://developer.android.com/develop/background-work/services/fgs/service-types)); `onTimeout(int)` "Added in API level 34": if the service "doesn't finish even after it's timed out, the app will be declared an ANR after a short grace period of several seconds" ([Service](https://developer.android.com/reference/android/app/Service#onTimeout(int))) |
| shortService, device 37 | both `onTimeout(int)` and `onTimeout(int,int)` (fgsType 2048) at **180.0 s** | `onTimeout(int,int)` "Added in API level 35"; from Android 15 it "will also be called when a foreground service of type ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE times out" ([Service](https://developer.android.com/reference/android/app/Service#onTimeout(int))) |
| shortService, devices 29/31 | 29: **no timeout** in a 240 s run (`review-a1-a29-full-short-240`); 31: 45 s only, not checked past 180 s | "The `Service.onTimeout()` callback doesn't exist on Android 13 and lower. If the same service runs on such devices, it doesn't receive a timeout, nor does it ANR." ([service-types](https://developer.android.com/develop/background-work/services/fgs/service-types)) |
| dataSync, devices 29, 31 and 34 measured (30, 32, 33 from the documentation) | **no limit**: 660 s runs on 29/31/34; on 34 a 60 s adb budget is ignored and the change id `FGS_INTRODUCE_TIME_LIMITS` does not exist (B1 re-run `review-b1-a34-combo-datasync-knob60` and A2's sync-suspended `review-a2-guarded-a34-combo-datasync-knob60` ran their full 150 s with the key still `60000` at the end; `review-a1-a34-combo-datasync-knob60` lost the key to a flag sync mid-run; `dumpsys platform_compat` has no such change id) — and AOSP says why the knob *cannot* exist there: android14's `ActivityManagerConstants` knows only `short_fgs_timeout_duration`, the `data_sync_fgs_timeout_duration` and `media_processing_fgs_timeout_duration` keys appear in android15 (`docs/aosp-android14-release-ActivityManagerConstants.java` vs `docs/aosp-android15-release-ActivityManagerConstants.java`) | The limit starts with Android 15: dataSync "has time limit of 6 hours starting from Android version Build.VERSION_CODES.VANILLA_ICE_CREAM", and since `onTimeout(int, int)` did not exist before, "it will never be called" there ([ServiceInfo](https://developer.android.com/reference/android/content/pm/ServiceInfo#FOREGROUND_SERVICE_TYPE_DATA_SYNC)) |
| dataSync, device 37 (target 37) | with a 60 s adb budget: `onTimeout(startId=1, fgsType=1)` 61 s after HOME, `stopSelf()` → no crash (`review-a1-a37-combo-datasync-knob60`; B1 re-run `review-b1-a37-combo-datasync-knob60` logged `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=1` 61.08 s after HOME, `EXPORT_ABORTED elapsedS=69`) | "If an app targets Android 15 or higher, the system places restrictions on how long certain foreground services are allowed to run while your app is in the background"; dataSync and mediaProcessing get "a total of 6 hours in a 24-hour period", tracked separately per type ([FGS timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout)); `dumpsys platform_compat`: `FGS_INTRODUCE_TIME_LIMITS; enableSinceTargetSdk=35` |
| mediaProcessing, device 37 (target 37) | with a 60 s adb budget: `onTimeout(startId=1, fgsType=8192)` 61 s after HOME (69 s after `startForeground`), `stopSelf()` → no crash (`review-a1-a37-combo-auto-knob60`; again with the fixed build, `review-a1-fixed-a37-combo-auto-knob60`, logging `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192`; and in B1, `review-b1-a37-combo-auto-knob60` → `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192`, `EXPORT_ABORTED elapsedS=69`, onTimeout 61.07 s after HOME) | same page; the budget resets "if the user brings the app to the foreground"; if the service does not call `stopSelf()` within a few seconds the system logs `RemoteServiceException: "A foreground service of type [service type] did not stop within its timeout"` (the service-types page calls it an ANR, the troubleshooting page an internal exception that crashes the app) |
| mediaProcessing, devices 29–34 | never reached: rejected on 34, never chosen below 35 | The service-types note that on Android 14 or lower a timed-out mediaProcessing service is cached immediately describes a case this recommendation never creates |

Observed vs documented start of the 6-hour clock: with a 60 s budget the timeout fired **61.05–61.11 s
after HOME** on the device clock — about 69–70 s after `startForeground`, never 60 s after it — in six
runs: mediaProcessing 61.05 s (`review-a1-a37-combo-auto-knob60`), 61.07 s
(`review-a1-fixed-a37-combo-auto-knob60`) and 61.07 s (`review-b1-a37-combo-auto-knob60`); dataSync
61.07 s (`review-a1-a37-combo-datasync-knob60`), 61.08 s (`review-b1-a37-combo-datasync-knob60`) and
61.11 s (`review-b1-a37-exhaust-1st-timeout`). Both ends are device log lines of the same run: HOME is
the `ActivityStartInterceptor: Starting home with component specified` line, the timeout the
`ActivityManager: FGS (<type>) timed out` line. (Earlier versions of this table gave 61.0–61.7 s; those
figures subtracted the host's second-truncated `home_sent=` from device times.) The two documentation
passages look contradictory — the timeout page says the service "has the full six hours after the app
goes to the background", the `Service.onTimeout(int,int)` reference says "that time counter begins as
soon as the foreground service starts" — and the platform source resolves it. The timer is first armed
from the service's start and its accumulated runtime, but when it fires while the app has been in the
TOP state within the last limit, it is re-armed to `lastTopTime + limit` instead of being delivered
([`ActiveServices.onFgsTimeout`, android15-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActiveServices.java),
local copy `docs/aosp-android15-release-ActiveServices.java`):

```java
if (lastTopTime != Long.MIN_VALUE && constantTimeLimit > (nowUptime - lastTopTime)) {
    …
    // The app was in the TOP state after the FGS was started so its time allowance
    // should be counted from that time since this is considered a user interaction
    …
    mAm.mHandler.sendMessageAtTime(msg, lastTopTime + constantTimeLimit);
```

`lastTopTime` is when the process last left TOP, which is about a second after the HOME key, not the
key itself. Review A2 recorded it (`processes_bg.txt`: `dumpsys activity processes` with the device
time just before it) and the timeout came 60.02 s after it in all seven runs that captured it, while
the process left TOP 1.04–1.10 s after the HOME line: mediaProcessing `review-a2-a37-combo-auto-knob60`
and `review-a2-guarded-a37-combo-auto-knob60`; dataSync `review-a2-guarded-a37-combo-datasync-knob60`,
`review-a2-a37-exhaust-{1st-timeout,2nd-foreground}` and `review-a2-guarded-a37-exhaust-{1st-timeout,2nd-foreground}`.
So the "extra second" is the app's own exit from TOP; the message itself fires on time.

The same source implements the documented reset at start: `fgsTypeInfo.reset()` when "the app is
currently in the TOP state or was in the TOP state after the time limit was exhausted previously" (or
the first start of the type was more than 24 h ago). If instead the budget was exhausted, the start
throws `ForegroundServiceStartNotAllowedException("Time limit already exhausted for foreground service
type …")` only when the aconfig flag `android.app.Flags.enableFgsTimeoutCrashBehavior()` is on; with the
flag off the platform logs the message as a WTF and resets the record anyway. The same flag decides
whether a service that gets `onTimeout` and does not stop is crashed (`onFgsCrashTimeout`: "Log a WTF
instead of crashing the app while the new behavior is gated") — both in the android15-release copy, lines
2459 and 3964. The API 37 image declares no such flag (absent from every `aconfig_flags.pb` on it, while
other FGS flags are present: `logs/a4-flag37.txt`), and `aflags` needs root there, so which way a given
Android 15–17 build behaves was not established here; the guide's advice (stop within seconds of
`onTimeout`, start from the foreground) is right either way. A start from an activity is always the
first case, so the exhausted-budget-then-foreground-start
runs (`review-b1-a37-exhaust-*`, and `review-a2-a37-exhaust-*`, where the second service then timed
out 60.0 s after its own `lastTopTime`) show that such a start is accepted and gets a full budget, but
they cannot isolate the reset: without it the re-arm above would give the same timeout. The
exception branch needs a start from the background, which this test app does not make. For the guide:
time the app spends in the foreground is not held against the service (the timeout cannot fire until
the limit has passed since the app last left the foreground), a user re-opening the app resets the
record, and the export should still call `stopSelf()` when it finishes instead of reasoning about the
clock.

The knobs themselves, checked against the platform source: namespace `activity_manager`, keys
`data_sync_fgs_timeout_duration` and `media_processing_fgs_timeout_duration`
(`KEY_DATA_SYNC_FGS_TIMEOUT_DURATION`, `KEY_MEDIA_PROCESSING_FGS_TIMEOUT_DURATION`), each defaulting to
`6 * 60 * 60_000` — "6 hours" is a default in code, not a constant in the docs
([`ActivityManagerConstants.java`, android15-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActivityManagerConstants.java),
local copy `docs/aosp-android15-release-ActivityManagerConstants.java`). The android14 copy of the same
file contains neither key (only `short_fgs_timeout_duration`), so on 34 `device_config put` accepts the
write — it accepts any key — and nothing reads it; that is the full explanation of the ignored 60 s
budget. Every key written during this investigation was deleted and read back: `null` (see
`timeout_knob.txt` in each knob case and `review-b1-a37-datasync-exhaust-reset/knob.txt`).

## Recommended declaration for the CraftBridge guide

```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_MEDIA_PROCESSING" />

<service
    android:name=".ExportService"
    android:foregroundServiceType="mediaProcessing|dataSync"
    android:exported="false" />
```

```kotlin
// Android 15+ (API 35): mediaProcessing — the type purpose-built for exports/transcoding.
// Android 10–14 (API 29–34): dataSync — measured on 29, 31 and 34; 30, 32 and 33 expected from
// Android's "Changes to foreground services", which changes nothing for it before Android 14.
// Always pass the type: on Android 14 the type-less call inherits the unknown mediaProcessing bit
// from this manifest and throws InvalidForegroundServiceTypeException.
val fgsType = if (Build.VERSION.SDK_INT >= 35) {
    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
} else {
    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
}
startForeground(NOTIFICATION_ID, notification, fgsType)   // Service method, API 29+

// API 35+: called when the type's 6-hour background budget runs out; stop within seconds.
override fun onTimeout(startId: Int, fgsType: Int) {
    cancelExport()   // the host's own cancellation
    stopSelf()
}
// When the export finishes or fails: stopSelf() — there is no callback below API 35.
```

This is the call the test app made. The notification it posts needs no permission to start the service: "Apps don't need to request the POST_NOTIFICATIONS permission in order to launch a foreground service", but on Android 13 and later, "if the user denies the notification permission, they still see notices related to foreground services in the Task Manager but don't see them in the notification drawer" ([notification permission](https://developer.android.com/develop/ui/views/notifications/notification-permission), read live on 2026-11-13 and checked again by the final review the same day; not saved, see the docs list in §Files). So a host app that wants the export's progress in the drawer also declares and requests `POST_NOTIFICATIONS`; the export runs either way. The tested APK declared it and `adb install -g` granted it, so a denied permission was not run. The Android 14 page names `ServiceCompat.startForeground()`
(androidx-core 1.12+) the best practice; with the two type values used here it should pass them
through unchanged, but `ServiceCompat` was not exercised in this investigation.

Why each piece, tied to evidence:

1. **`mediaProcessing` is API 35, not 34.** Reference: `FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING` "Added in API level 35", constant 8192 ([ServiceInfo](https://developer.android.com/reference/android/content/pm/ServiceInfo#FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING)); timeout page: "(The `mediaProcessing` foreground service type was added in Android 15.)" ([link](https://developer.android.com/develop/background-work/services/fgs/timeout)). Runtime confirms: accepted on 37 (`a37-full-mediaproc`, `review-a1-a37-combo-mediaproc`), **prohibited on 34 even when declared in the manifest** (`a34-full-mediaproc`, `a34-full-ts34-mediaproc`, `review-a1-a34-combo-mediaproc`). So the runtime switch threshold must be **≥ 35 — Android 14 must fall back too**. The same reference says the type "can be also used on prior android versions (just like other new foreground service types can be used)"; that is *not* true on the Android 14 platform for apps targeting 34+ (the `a34-*` failures); it holds on API 29/31 only because those platforms contain no enforcement (`a29-full-mediaproc`, `a31-full-mediaproc`). The data-transfer guide says the type "is available if your app targets Android 15 or higher" ([link](https://developer.android.com/about/versions/15/changes/datasync-migration)); on the device it is the platform level that counts — a targetSdk-34 build was accepted on 37 (`a37-full-ts34-mediaproc`), though using the constant, and the `mediaProcessing` flag in the manifest's `android:foregroundServiceType`, needs compileSdk ≥ 35: `platforms/android-34/data/res/values/attrs_manifest.xml` has no `mediaProcessing` flag, and android-35, -36, -36.1 and -37.0 have one, so aapt2 cannot resolve the recommended manifest below 35.
2. **`dataSync` is the fallback for 29–34**: measured on 29, 31 and 34 (45 s on 29/31/34/37; 660 s on 29/31/34), and expected on 30, 32 and 33 from the documentation (`docs/fgs-changes.txt`: before Android 14 the only type rules are location from 10 and camera and microphone from 11). Its documented intended uses explicitly include "Import or export operations" and "Local file processing" ([service-types Data sync](https://developer.android.com/develop/background-work/services/fgs/service-types)) — an EffectCraft export fits. Needs `FOREGROUND_SERVICE_DATA_SYNC` on 34+ (observed `SecurityException`, `a34-noperm-datasync`); not checked before 34 (`a29/a31-noperm-datasync` ✅). On Android 10–14 it has no time limit at all (Timeouts table); its 6-hour limit applies only from Android 15 on, where this recommendation does not use it. Current guidance does not deprecate it, but steers transfers elsewhere: its Alternatives section links the data-transfer guide, which lists `mediaProcessing` for "Encoding or decoding a media file" and recommends user-initiated data-transfer jobs for user-started network transfers; Play policy says the same (point 9). The Android 14 platform carries two compat switches for `dataSync` (`docs/aosp-android14-release-ForegroundServiceTypePolicy.java` 125–145, 296–304), both off by default and overridable (`dumpsys platform_compat` on the API 34 boot), and every other run here was in the default state. Only one turns `dataSync` off: with `FGS_TYPE_DATA_SYNC_DISABLED_CHANGE_ID` (255659651) enabled for the app, `startForeground(…, DATA_SYNC)` meets `FGS_TYPE_POLICY_CHECK_DISABLED` and throws `InvalidForegroundServiceTypeException … has been prohibited` (android14 `ActiveServices` 2573–2588), and catching it ends quietly (`final-v7-a34-combo-datasync-compat-disabled`), so the guide says the host app catches that exception like any other start failure. The other, `FGS_TYPE_DATA_SYNC_DEPRECATION_CHANGE_ID` (255039210), only marks the type deprecated: `checkForegroundServiceTypePolicy` returns `FGS_TYPE_POLICY_CHECK_DEPRECATED` (`ForegroundServiceTypePolicy.java` 1400–1402), which `ActiveServices` 2564–2571 logs and lets through, and with it enabled the export started and finished (`final-v7-a34-combo-datasync-compat-deprecated`: `W/ActivityManager: Starting FGS with type dataSync code=2`, then `EXPORT_DONE elapsedS=45`).
3. **One manifest string for every level**: `mediaProcessing|dataSync` installs on all four levels (`install_rc=0` everywhere, incl. API 29 where the bit is unknown). The declared-but-unused type is harmless only while the explicit type is passed: the type-less start fails on 34 with this manifest (`review-a1-a34-combo-none`). Do **not** add `shortService` to the combo — the runtime type must be a subset of the declared attribute, and passing one that is not crashes the app even when the exception is caught (`a34/a37-combo-short-notdeclared`); the reference also says a `shortService` bit combined with other types is ignored anyway.
4. **Never ship an untyped export FGS**: `MissingForegroundServiceTypeException` on 34/37 is target-SDK-gated — proven with the three builds: ts37/ts34 fail, ts33 passes on the same device (`a34-noneflavor-none`, `a34-none-ts34-none`, `a34-none-ts33-none`). A guide recommendation of "no type on old devices" would break the moment a host app bumps targetSdk.
5. **`specialUse` is the documented escape hatch** (works on 29/31/34/37 incl. 660 s runs, `a34/a37-full-specialuse-long`) but requires `FOREGROUND_SERVICE_SPECIAL_USE` + `<property android:name="android.app.PROPERTY_SPECIAL_USE_FGS_SUBTYPE" …>` (used in the `full` flavor per the [service-types page](https://developer.android.com/develop/background-work/services/fgs/service-types)); "These values and corresponding use cases are reviewed when you submit your app in the Google Play Console." Keep it as a plan-B note for host apps rejected for dataSync, not the default.
6. **`shortService` must never be used for exports**: 180 s observed platform timeout on 34 and 37, and an ANR is documented if the service does not stop within seconds of `onTimeout`.
7. **Handle `onTimeout(startId, fgsType)`** (API 35+) by cancelling and calling `stopSelf()` within seconds: the reference says "if it fails to do so, the app will crash" ([Service](https://developer.android.com/reference/android/app/Service#onTimeout(int,%20int))); handled this way it caused no crash for mediaProcessing or dataSync (`review-a1-a37-combo-auto-knob60`, `…-datasync-knob60`). The one-argument `onTimeout(startId)` exists only for `shortService`, which the guide does not use; the reference adds that developers "do not need to implement both of the callbacks" on Android 15 and later. Below API 35 nothing calls back, so the service must stop itself when the export ends.
8. **Start the export FGS while the app is in the foreground** (user initiates the export). "Apps that target Android 12 (API level 31) or higher can't start foreground services while the app is running in the background, except for a few special cases", throwing `ForegroundServiceStartNotAllowedException` ([background-start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)). Once started, running in the background is fine (660 s after HOME at every level; `resumed_bg.txt` shows the launcher on top while `dumpsys_bg.txt` shows `isForeground=true`). On 15+ a user-started service also gets the full 6-hour budget, which only counts background time (timeout page, option 3). Not exercised: a start from the background.
9. **Google Play declarations for both types.** "When your apps target Android 14 and above, you’ll need to declare any foreground service types that you use in a new declaration on the App content page ( Monitor and improve > App content ) in Play Console" ([Play Console help 13392821](https://support.google.com/googleplay/android-developer/answer/13392821)); the developer page calls the same page "Policy > App content" ([service-types](https://developer.android.com/develop/background-work/services/fgs/service-types)), so a guide should name both paths. "For each foreground service type you declare, you’ll need to do the following": "Provide a description of the app functionality that is using each foreground service type", "Describe the user impact if: the task is deferred by the system (does not start immediately); and/or the task is interrupted by the system (paused and/or restarted)", "Include a link to a video demonstrating each foreground service feature" and "Choose your specific use case for each foreground service type" ([same page](https://support.google.com/googleplay/android-developer/answer/13392821)). The combo manifest therefore needs **two** declarations, not one. Play's use-case rows that fit an export: under `TYPE_DATA_SYNC`, "Local processing : Import or export" ("Includes migration of files from SD card"; video: "Show the user manually triggering an import or export of files locally") and "Local processing : Other" ("Use for specifically user-initiated work and not for regular system or server-initiated tasks. For example, encryption, resizing."); for network transfers the page adds "we recommend using the user-initiated data transfer job instead of the dataSync foreground service type", which does not apply to a local render. Under `FOREGROUND_SERVICE_MEDIA_PROCESSING`, "Media transcoding" ("Performing time-consuming operations on media assets, like converting media to different formats"; video: "Show the user initiating the app converting or transcoding a video or audio file to a different format"). A user-tapped EffectCraft export fits "Media transcoding" on 15+; on 10–14 the host app picks the dataSync row that matches how its UI presents the export ("Import or export" if it is framed as exporting a file, "Other" if as local processing). The guide should tell host apps to file both declarations and to record the user action that starts the export for the video.
10. **Keep the CPU running with the screen off** (documented, not exercised — the emulator never suspends): a foreground service does not by itself stop the device from suspending. The keep-awake guide asks: "Would it be detrimental to the user experience if the device suspends while the foreground service is running and the device screen is off?" and answers "If Yes, you might need to use a wake lock. However, you should still check to see if you're already using an API or doing an action that declares a wake lock on your behalf, as discussed in Actions that keep the device awake." That section ("If your app is doing any of the following, you don't need to set a wake lock yourself") lists the cases where something else already holds one: "If you're using task scheduling APIs or libraries such as WorkManager, JobScheduler, or DownloadManager, the system or library acquires a wake lock that is attributed to your app", and "If you're using Media3 ExoPlayer, you can use ExoPlayer.setWakeMode() to have the player set a wake lock for you" ([keep the device awake](https://developer.android.com/develop/background-work/background-tasks/awake)). So the guide's rule should be: if the export runs inside a WorkManager long worker (or a Media3 transcode), say *no extra wake lock needed*; if it is a plain started service doing its own transcoding, hold a `PARTIAL_WAKE_LOCK` (with the `WAKE_LOCK` permission) for exactly the export's duration and release it in the same place it calls `stopSelf()` — a wake lock does not extend any FGS allowance, and one left held after the service stops just drains the battery. Verify on a physical phone.

## What the emulator cannot show (needs a physical phone)

* **CPU suspend with the screen off.** The emulator never suspends; on a phone an FGS without a wake lock can stall when the screen turns off (point 10).
* **OEM aggressive process kill** (Xiaomi/Huawei/Samsung battery managers, "autostart" managers) that can kill even a legitimate FGS — the AOSP emulator never killed a correctly-foregrounded service, which is exactly what we observed.
* **Thermal throttling** of a minutes-long CPU/GPU export: the export gets slower on a hot phone, so a real export takes longer than its emulator time; only the 6-hour budget on 15+ bounds that.
* **Real Doze / App Standby buckets / battery saver** behaviour over long unattended periods (hours) on battery; the emulator is always charging.
* The **full 6 h/24 h budget** end-to-end (only exercised with adb-shortened 60 s budgets). After a
  dataSync service had burned a 60 s budget and timed out, a new dataSync start **from the foreground**
  was accepted and ran (`review-b1-a37-exhaust-1st-timeout` → `FGS_ON_TIMEOUT`, then
  `review-b1-a37-exhaust-2nd-foreground` → `STARTFOREGROUND_OK requested=dataSync` → `EXPORT_DONE`;
  again in `review-a2-a37-exhaust-*`, where the second service ran into a fresh 60 s timeout). That is
  what the docs promise — "you cannot start another dataSync foreground service **unless the user has
  brought your app to the foreground (which resets the timer)**"
  ([FGS timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout)) — but
  it does not isolate the reset itself (see the clock note under Timeouts). Not shown: the 24 h window
  and the documented `ForegroundServiceStartNotAllowedException` when a **background** start meets an
  exhausted budget.
* **Play Console review** of the dataSync/mediaProcessing declarations and of `specialUse` justification text.
* Notification shade behavior with real `POST_NOTIFICATIONS` denial UX, keyguard, heads-up state (our runs had it auto-granted via `adb install -g`).
* "Swipe to dismiss from recents" kill behavior on OEM skins, and reboot-resume/`START_STICKY` re-delivery under real memory pressure.

## Not exercised (stated so the record is honest)

* `mediaProcessing` passed at runtime on API 31 — now covered for the combo APK in review B1
  (`review-b1-a31-combo-mediaproc` → `EXPORT_DONE`, `typeBit=8192`; the full flavor was already covered
  by `a31-full-mediaproc`). On 29 the rebuilt combo APK was accepted too
  (`review-a1-fixed-a29-combo-mediaproc`, `review-b1-a29-combo-mediaproc`, `typeBit=8192`). The `auto`
  logic never passes it below 35.
* App *not* declaring the base `FOREGROUND_SERVICE` permission (documented requirement: the service-types page requires the type permission "in addition to requesting the `FOREGROUND_SERVICE` permission", [link](https://developer.android.com/develop/background-work/services/fgs/service-types)); not tested empirically.
* Android 11, 12L and 13 (API 30, 32, 33) — not run; the owner chose not to download those three system images. Type and per-type-permission enforcement starts with Android 14 ([changes to foreground services](https://developer.android.com/develop/background-work/services/fgs/changes), saved as `docs/fgs-changes.html` and `docs/fgs-changes.txt`), so all three are expected, from that documentation, to behave like 29/31 here (API 33 like 31, not like 34); 31+ adds only the background-start restriction, which a foreground start does not meet.
* `ServiceCompat.startForeground`, a wake lock, a start from the background, an `onTimeout` handler that does *not* stop (documented crash/ANR; flag-gated in the android15 source, see the clock note under Timeouts), and `stopForeground` re-promotion and the type-change rules.

## Artifacts

```
app/                     test app source (./gradlew assembleDebug — verified BUILD SUCCESSFUL)
apk/, apk-ts34/, apk-ts33/      first build (a29/a31 matrix)
apk2/, apk2-ts34/, apk2-ts33/   final build-side APKs (34/37 matrix, combo-final, review-a1 re-runs)
apk-a1/, apk-a1-ts34/, apk-a1-ts33/  app/ after the review-A1 fixes (review-a1-fixed-* cases)
apk-b1/, apk-b1-ts34/, apk-b1-ts33/  review B1's clean rebuild of that same source (md5s identical to apk-a1*)
evidence/apk-md5.txt     checksums of every APK set
evidence/a<api>-<case>/  build-side cases; evidence/review-a1-*/ review re-runs;
                         evidence/review-b1-*/ review B1 re-runs (CASE_TAG-prefixed queue runs)
evidence/review-a1-build-compare/  manifest and bytecode comparisons of the three builds
run_case.sh              one-scenario harness (own-pid log matching, in-background dumpsys, crash-safe)
queue.sh                 one device through a case set (matrix | review-a1 | smoke | amendments |
                         timeouts) from a required APK directory; CASE_TAG=<tag> re-labels the output dirs so a later round never
                         overwrites an earlier one; timeout knobs suspend device_config sync, and every
                         key is read back before and after deletion
summarize.sh             one block per case, own-process log lines only
docs/                    saved copies of the developer.android.com / Play Console pages cited, Changes to
                         foreground services included (`fgs-changes.html`, its text in `fgs-changes.txt`;
                         FGS troubleshooting and the notification-permission page, read live on
                         2026-11-13, were saved on 2026-11-14 as `fgs-troubleshooting.html` and
                         `notification-permission.html`, their text in `.txt`, with the quoted
                         sentences still in them; the
                         ServiceInfo reference was saved twice, as `serviceinfo-ref.txt` and as
                         `docs_tmp.html`), plus docs/aosp-*.java — AOSP `ActiveServices`
                         (android10/14/15-release), `ActivityManagerConstants` (14/15),
                         `ForegroundServiceTypePolicy` (14) and `ServiceRecord` (15), and
                         SettingsProvider's `DeviceConfigService` (14/15/16),
                         `SettingsToPropertiesMapper` (14/16) and `SettingsState` (16) behind the
                         device_config sync guard, fetched with `?format=TEXT | base64 -d` from
                         android.googlesource.com; plus two stray captures from early rounds:
                         `fgs-guide.txt` (foreground-services overview page, not cited) and
                         `v12-changes.html`/`.txt` (a failed fetch that saved the 404 page)
logs/                    emulator + queue logs (b1-* = review B1 runs and cross-checks; the b1-run*.sh /
                         b1-reset37.sh files are B1's transcripts, now covered by queue.sh sets; a2-*, b2-*,
                         a3-*, … = later rounds; <round>-pids.txt lists the emulators a round started)
REVIEW-<round>.md        one per review round (A1, B1, A2, B2, …): claims checked, re-runs
                         (CASE_TAG=review-<round>), documentation read, defects fixed, end state
```

The plan's copy of this evidence, `plan/evidence/v7/` in the CraftBridge repository, holds everything above but `docs/` (another product's pages and sources), each case's `logcat_full.txt` and the two boot-loop logcats in `logs/`, which its `MANIFEST.md` lists with their SHA-256s; their copies are in the owner's work archive outside the repository, `<archive>/v7/`. The repository is public, so in the emulator logs the Mac's home folder is written `$HOME`, its user `$USER`, and the adb key the emulator was handed `<adb key>`.

Case directory names are the labels in the tables above (`a29-…`, `a31-…`, `a34-…`, `a37-…`, and `review-<round>-…` for each review round).
Docs read 2026-11-13 from: [foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types), [FGS timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout), [Android 14 FGS-types-required](https://developer.android.com/about/versions/14/changes/fgs-types-required), [Android 15 FGS-types changes](https://developer.android.com/about/versions/15/changes/foreground-service-types), [data-transfer options](https://developer.android.com/about/versions/15/changes/datasync-migration), [changes to foreground services](https://developer.android.com/develop/background-work/services/fgs/changes), [FGS troubleshooting](https://developer.android.com/develop/background-work/services/fgs/troubleshooting), [background-start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start), [keep the device awake](https://developer.android.com/develop/background-work/background-tasks/awake), [ServiceInfo reference](https://developer.android.com/reference/android/content/pm/ServiceInfo), [Service reference](https://developer.android.com/reference/android/app/Service), [Play Console FGS requirements](https://support.google.com/googleplay/android-developer/answer/13392821). Platform source read the same day from android.googlesource.com (`?format=TEXT`, base64-decoded into `docs/`): [`ActiveServices.java` android10](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android10-release/services/core/java/com/android/server/am/ActiveServices.java), [android14](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/services/core/java/com/android/server/am/ActiveServices.java), [android15](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActiveServices.java); [`ForegroundServiceTypePolicy.java` android14](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/core/java/android/app/ForegroundServiceTypePolicy.java); [`ActivityManagerConstants.java` android14](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/services/core/java/com/android/server/am/ActivityManagerConstants.java), [android15](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActivityManagerConstants.java); [`ServiceRecord.java` android15](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ServiceRecord.java).
