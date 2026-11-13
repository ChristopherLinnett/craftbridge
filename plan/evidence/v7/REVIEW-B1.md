# Review round B1 — CraftBridge V7 (#30): foreground-service type for a long export

Build side, round B1, working alone in `/tmp/craftbridge-v7`. Inputs: the B1 brief
(`/tmp/craftbridge-amend/v7-b1-brief.md`, read only) and the original brief, `RESULT.md` as handed over
after round A1, `REVIEW-A1.md`, `app/`, every APK set, `evidence/`, `run_case.sh`/`queue.sh`/`summarize.sh`.
This round re-did the work rather than auditing the write-up: it rebuilt the app, re-ran the three
amendments on four API levels, read the platform source behind the two behaviours the report had left as
observations, and re-checked every remaining claim against the evidence on disk. Every runtime statement
cites an evidence directory; every documentation statement cites the page (or AOSP file) it was read
from on 2026-11-13.

**Verdict: 12 defects found and fixed (D19–D30, none left open); the recommendation and all three of round A1's amendments stand — B1 changed no conclusion, only the accuracy of the statements and the harness behind them.**

Round A1's 18 defects were re-checked, not re-litigated: all 18 still hold (the voided `a29-noneflavor-long`,
the `onTimeout` parameter naming, the `noperm` evidence, the merged manifests, the mislabelled-emulator
process note, etc.). Nothing A1 wrote had to be reverted. One cosmetic difference is noted in D24.

## 1. Claims checked

`logs/b1-verify.sh` (output `logs/b1-verify.txt`) rebuilds, per case, what `case.txt` should have said:
APK path + recomputed md5, device SDK, target SDK, run pid, outcome, and the outcome markers taken from
that pid's own log lines. Results of the cross-checks:

| Check | Method | Result |
|---|---|---|
| Every build-side case installed and on the right device | `install.txt` + `case.txt`/`SERVICE_CREATED sdk=` for all 49 `a<api>-*` cases | 49/49 `Success`, `device_sdk` matches the `a<api>-` label |
| APKs match their recorded checksums | `md5` vs `evidence/apk-md5.txt` | all match, incl. the B1 rebuild (D28) |
| Caught exceptions really are caught | `grep -c "FATAL EXCEPTION"` in `a34-full-none`, `a34-noneflavor-none`, `a34/a37-noperm-*`, `review-b1-a34-combo-none`, `review-b1-a37-combo-none` | 0 in all; only the not-a-subset case crashes (as RESULT.md says) |
| Prompt `stopSelf()` avoids the shortService ANR | `grep "ANR in"` in the 34/37 shortService timeout cases | 0 |
| Backgrounded proof | `dumpsys_bg.txt` taken after HOME while `resumed_bg.txt` shows the launcher resumed | `isForeground=true`, `types=00000001` (34), `types=0x00002000` (37) |
| `dumpsys_late.txt` is `(nothing)` after the export | spot check of the long runs | as claimed |
| `SHORT_SERVICE will be ignored` log line | `a34-full-none/logcat_full.txt` | present; `dumpsys_early.txt` `types=40002001` |
| Throughput recomputed from the logs | `TICK elapsedS=60 fileKiB=38208`, `EXPORT_DONE … elapsedS=660 fileKiB=417280` | **≈630 KiB/s, not ~61 kB/s** → D19 |
| `am compat enable FGS_INTRODUCE_TIME_LIMITS` on 34 | `review-a1-a34-combo-datasync-knob60/timeout_knob.txt`, `review-b1-*` | `Unknown or invalid change`; id absent from `dumpsys platform_compat`; on 37 the same dumpsys shows `ChangeId(317799821; name=FGS_INTRODUCE_TIME_LIMITS; enableSinceTargetSdk=35)` |
| Recommended snippet == tested code | read of `ExportService.kt` / `MainActivity.kt` vs RESULT.md's snippet | same `resolveType`, `auto = SDK_INT>=35 ? mediaProcessing : dataSync`, both `onTimeout` overrides with correct parameter names, unknown name throws; no duplicated code, single `tick()`/`writeUntilDone()` loop |
| `<service>` declared once, flavour manifests | `aapt2 dump xmltree` of every B1 APK | one `<service>` per APK; combo `0x2001`, full `0x40002001` + property, none → no attribute, noperm `0x40002001` with only `FOREGROUND_SERVICE` + `POST_NOTIFICATIONS` |
| Builds clean | `logs/b1-build-*.log` for `assembleDebug`, `-PtargetSdk=34`, `-PtargetSdk=33` | `BUILD SUCCESSFUL`, **zero warning lines** |
| Environment table | AVD `config.ini`, `sysctl`, `getprop` | 18 cores / 64 GiB / Apple M5 Max; images `android-29/31/34 google_apis arm64-v8a`, `android-37.2 google_apis_playstore_ps16k`; 37 reports `ro.build.version.release=17`, sdk 37 |
| `noperm` permission evidence still exists | `a34/a37-noperm-*` logs + `aapt2` dump of `apk-b1/fgslab-noperm-ts37.apk` (md5 `15b2745f…`, identical to the APK those cases installed) | `SecurityException … requires permissions: all of the permissions allOf=true [android.permission.FOREGROUND_SERVICE_DATA_SYNC]` (and `_SPECIAL_USE`); on 34 `mediaProcessing` fails the type check first — as RESULT.md already said |

## 2. Cases re-run (review B1)

Source unchanged; `apk-b1*` is a clean rebuild whose four md5s equal `apk-a1*` exactly. `queue.sh` ran
end to end three times — 34 (`review-a1` set, 7 cases), 37 (same set, 7 cases), 31 (`smoke`, 3 cases) —
with `CASE_TAG=review-b1`, plus single `run_case.sh` runs (3 more on 31, 3 on 29, the 34/37 long, knob
and exhaustion cases). `evidence/review-b1-*` now holds 28 directories: 24 scenarios (tabulated below
and in RESULT.md §Review-B1 re-runs), one knob-transcript directory
(`review-b1-a37-datasync-exhaust-reset/knob.txt`), two harness self-checks
(`review-b1-harness-smoke-*`, D29) and one voided attempt (`review-b1-a29-noperm-datasync`, whose
`adb install` returned `Can't find service: package` because that device was mid-shutdown; labelled
`VOID-review-b1.txt`, supports no claim).

**Amendment 1 — the explicit type is mandatory (re-run on 34, plus 29/31/37 as the brief asked):**

| Case | Device | Result |
|---|---|---|
| `review-b1-a34-combo-none` | 34 | ❌ `InvalidForegroundServiceTypeException: Starting FGS with type unknown … targetSDK=37 has been prohibited` — reproduced, no `FATAL` |
| `review-b1-a34-combo-mediaproc` | 34 | ❌ same exception with the type passed explicitly |
| `review-b1-a34-combo-short-notdeclared` | 34 | `IllegalArgumentException … not a subset … 0x00002001` (caught) → `RemoteServiceException$ForegroundServiceDidNotStartInTimeException` 186 ms later |
| `review-b1-a37-combo-none` | 37 | ✅ accepted (`types=0x00002001`) → `EXPORT_DONE` — on 35+ the unknown bit is a real type, so the type-less call works |
| `review-b1-a37-combo-short-notdeclared` | 37 | same not-a-subset throw → same nested crash class |
| `review-b1-a29-combo-none`, `review-b1-a31-combo-none` | 29, 31 | ✅ accepted → `EXPORT_DONE` (no type policy below 34) |
| `review-b1-a29-combo-short-notdeclared` | 29 | same not-a-subset throw → `android.app.RemoteServiceException` (the subclass does not exist on 10) |
| `review-b1-a31-combo-short-notdeclared` | 31 | same throw → `android.app.ForegroundServiceDidNotStartInTimeException` |

So: on every level the runtime type must be a subset of the declared attribute; on 34 the type-less call
with the recommended manifest is fatal to the feature; on 37 it works — which is exactly what
RESULT.md now says, and why the explicit type is the only portable form.

**Amendment 2 — time limits exist only from Android 15 (re-run + source):**

| Case | Device | Result |
|---|---|---|
| `review-b1-a34-combo-datasync-knob60` | 34 | ran its full 150 s; key `get`→`60000`, `delete`→"Successfully deleted", read back `null`; no such change id in `dumpsys platform_compat` |
| `review-b1-a37-combo-datasync-knob60` | 37 | `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=1` 61.7 s after HOME (69.9 s after `startForeground`), `EXPORT_ABORTED elapsedS=69`, no crash; key deleted → `null` |
| `review-b1-a37-combo-auto-knob60` | 37 | `FGS_ON_TIMEOUT_2ARG startId=1 fgsType=8192` 61.6 s after HOME, `elapsedS=69` |
| `review-b1-a37-exhaust-1st-timeout` / `-2nd-foreground` | 37 | budget burned → timeout; a **foreground** start of the same type immediately after is accepted and completes → documented reset confirmed |
| `review-b1-a34-full-short-timeout` | 34 | `FGS_ON_TIMEOUT_1ARG` at 180.0 s (third run of the 180 s figure) |
| `review-b1-a34-combo-datasync`, `-combo-auto-long`; `review-b1-a37-combo-datasync`, `-combo-mediaproc`, `-combo-auto-long` | 34, 37 | `EXPORT_DONE`, incl. 660 s runs `fileKiB=419392` (34, dataSync) and `413440` (37, mediaProcessing) |
| `review-b1-a31-combo-auto`, `-combo-mediaproc`, `-combo-none`, `-combo-bad-type-name`; `review-b1-a29-combo-none`, `-combo-mediaproc` | 31, 29 | as tabulated in RESULT.md §Review-B1 re-runs |

**Amendment 3** (two Play Console declarations + wake lock) is a documentation item: re-read and
re-quoted verbatim this round (§3, D26, D27).

## 3. Documentation read (2026-11-13)

Developer documentation (local copies in `docs/`): FGS service types — `mediaProcessing` "Service for
performing time-consuming operations on media assets, like converting media to different formats. The
system allows this service a limited time to run; under normal circumstances, this time limit would be
6 hours out of every 24"; dataSync intended uses incl. "Import or export operations" and "Local file
processing"; specialUse "These values and corresponding use cases are reviewed when you submit your app
in the Google Play Console." ([link](https://developer.android.com/develop/background-work/services/fgs/service-types)).
FGS timeouts — "…the full six hours after the app goes to the background" and "If your app's dataSync
foreground services have run for 6 hours in the last 24, you cannot start another dataSync foreground
service unless the user has brought your app to the foreground (which resets the timer)"
([link](https://developer.android.com/develop/background-work/services/fgs/timeout)). `Service.onTimeout(int)`
/ `onTimeout(int,int)` — "Added in API level 34"/"35", "that time counter begins as soon as the
foreground service starts"
([link](https://developer.android.com/reference/android/app/Service#onTimeout(int))).
Keep the device awake — "Would it be detrimental to the user experience if the device suspends while the
foreground service is running and the device screen is off? … **If Yes**, you might need to use a wake
lock. However, you should still check to see if any of the following, you don't need to set a wake lock
yourself" and the list naming WorkManager/JobScheduler/DownloadManager/Media3 ExoPlayer
([link](https://developer.android.com/develop/background-work/background-tasks/awake)).
Play Console 13392821 — "you'll need to declare any foreground service types that you use in a new
declaration on the App content page ( Monitor and improve > App content ) in Play Console", "For each
foreground service type you declare, you'll need to do the following: … Include a link to a video
demonstrating each foreground service feature", `TYPE_DATA_SYNC` → "Local processing : Import or export",
"Use for specifically user-initiated features", "we recommend using the user-initiated data transfer job
instead of the dataSync foreground service type", `FOREGROUND_SERVICE_MEDIA_PROCESSING` → "Media
transcoding … converting media to different formats"
([link](https://support.google.com/googleplay/android-developer/answer/13392821)).

Platform source, fetched with `?format=TEXT | base64 -d` into `docs/` (this is what turned two
"observed, undocumented" items into explained ones):

* [`ActiveServices.java`, android14-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/services/core/java/com/android/server/am/ActiveServices.java)
  — the unknown-type rule: *"If the service has declared some unknown types which might be coming from
  future releases, and if it also comes with the specialUse, then it'll be deemed as the specialUse and
  we ignore this unknown type. Otherwise, it'll be treated as an invalid type."*
* [`ForegroundServiceTypePolicy.java`, android14-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/core/java/android/app/ForegroundServiceTypePolicy.java)
  — per-bit policy table, `// Unknown type, fallback to the defaultToType`, and the switch in
  `ActiveServices.validateForegroundServiceType` that turns one policy code into one exception:
  `FGS_TYPE_POLICY_CHECK_DISABLED` → `InvalidForegroundServiceTypeException … has been prohibited`,
  `FGS_TYPE_POLICY_CHECK_PERMISSION_DENIED_ENFORCED` → `SecurityException … requires permissions`. One
  bit yields exactly one code, which is why `a34-noperm-mediaproc` fails as an unknown type on 14 and as
  a missing permission on 37.
* [`ActiveServices.java`, android15-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActiveServices.java)
  — `onFgsTimeout()`: *"The app was in the TOP state after the FGS was started so its time allowance
  should be counted from that time since this is considered a user interaction"*, re-arming at
  `lastTopTime + constantTimeLimit`; and at start, `fgsTypeInfo.reset()` when the app is TOP (or was TOP
  after a previous exhaustion), else `ForegroundServiceStartNotAllowedException("Time limit already
  exhausted for foreground service type …")`.
* [`ActivityManagerConstants.java`, android15-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/services/core/java/com/android/server/am/ActivityManagerConstants.java)
  vs [android14-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android14-release/services/core/java/com/android/server/am/ActivityManagerConstants.java)
  — `data_sync_fgs_timeout_duration` / `media_processing_fgs_timeout_duration` exist only in the android15
  copy, both `6 * 60 * 60_000`; android14 has only `short_fgs_timeout_duration`. Namespace
  `activity_manager` (used by `device_config`, confirmed by the working `put`/`get`/`delete` on 37).
* [`ActiveServices.java`, android10-release](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android10-release/services/core/java/com/android/server/am/ActiveServices.java)
  + `ServiceRecord.java` (android15) as supporting reads.

## 4. Defects found and fixed

Files changed this round: `RESULT.md`; `run_case.sh`; `queue.sh`; `evidence/apk-md5.txt`; new
`docs/aosp-*.java` (7 files: `ActiveServices` for android10/14/15-release, `ActivityManagerConstants`
and `ForegroundServiceTypePolicy` for android14/15-release, `ServiceRecord` for android15) and
`docs/play-console-fgs.{html,txt}`, `docs/awake.{html,txt}`; new `evidence/review-b1-*` (28 directories:
24 scenarios, the exhaustion knob transcript, 2 harness self-checks, 1 voided attempt); `logs/b1-*`
(builds, emulator boots, queue runs, `b1-verify.{sh,txt}`); new `apk-b1/`, `apk-b1-ts34/`,
`apk-b1-ts33/`; this file. No app source change was needed (the rebuild is byte-identical to `apk-a1`).

**RESULT.md accuracy**

* **D19 — the export's throughput was off by ~10×.** "≈40 ms of `sqrt` math + 64 KiB append per tick,
  ~61 kB/s sustained" contradicted the app's own `TICK` lines (38,208 KiB in 60 s) and `EXPORT_DONE`
  (417,280 KiB in 660 s). Fix: "≈630 KiB/s sustained" with both measurements inline.
* **D20 — "no documentation describes it" was wrong, and the report stopped at an observation.** The
  Android 14 asymmetry (`full` manifest accepts the unknown bit for a type-less call, `combo` does not) is
  implemented in `ActiveServices` + `ForegroundServiceTypePolicy` and quoted in RESULT.md now, together
  with the two consequences: an unknown bit is only survivable next to `specialUse`, and the fallback is
  not documented behaviour a guide may lean on. The same source also let me replace the report's loose
  "the type check runs before the permission check" note for `a34-noperm-mediaproc` with what the code
  actually does: `validateForegroundServiceType` maps one policy code to one exception, and an unknown bit
  yields `DISABLED`, so the missing permission is never the failure on 14 — while on 37 the same noperm
  build fails with `SecurityException` (`a37-noperm-mediaproc`).
* **D21 — "no type enforcement at all" overreached** (API 29/31 headers, verdict paragraph). Those levels
  have no *type policy*, but they do enforce the manifest-subset rule; RESULT.md now says which of the
  four checks exist per level, backed by the four new 29/31 cases.
* **D22 — the fatal exception was reported as one class name.** It is `RemoteServiceException` on 29,
  `ForegroundServiceDidNotStartInTimeException` on 31, `RemoteServiceException$ForegroundServiceDidNotStartInTimeException`
  on 34/37, always 55–190 ms after the caught throw. Fixed with citations for all four levels.
* **D23 — the two conflicting doc passages about when the 6-hour clock starts were left unreconciled**
  after three measurements. Now resolved from `ActiveServices.onFgsTimeout` (background-only accounting +
  re-arm at `lastTopTime + limit`), with six measured runs (61.0–61.7 s after HOME, ~69–70 s after
  `startForeground`) and the practical consequence for the guide.
* **D24 — the "Android 14 ignores the budget" claim had no reason attached.** Added the AOSP key/default
  evidence (android14 has no such keys) and the B1 read-backs. Cosmetic difference worth recording: on
  round A1's 34 session `device_config delete` printed `Failed to delete …`, in B1 it printed
  `Successfully deleted … from activity_manager` and the key read back `null` both times; the outcome
  (key absent) is the same, and RESULT.md now quotes B1's transcript.
* **D25 — two "Not exercised" items were stale.** Runtime `mediaProcessing` on API 31 with the combo APK
  and the budget reset after exhaustion are now exercised (`review-b1-a31-combo-mediaproc`,
  `review-b1-a37-exhaust-*`) and cited where they belong.
* **D26 — the Play Console paragraph paraphrased.** Replaced with verbatim quotes, both menu paths
  (Play Console says "Monitor and improve > App content", the developer page says "Policy > App
  content"), the four per-type requirements, and the two use-case rows that the EffectCraft export maps
  onto. Also states explicitly that the combo manifest needs **two** declarations.
* **D27 — the wake-lock advice skipped the documented check-first rule** (WorkManager/JobScheduler/
  DownloadManager/Media3 already hold a wake lock). RESULT.md now branches: extra `PARTIAL_WAKE_LOCK`
  only for a plain started service, and drops an unsupported implication that a wake lock interacts with
  the FGS time limit.
* **D28 — artifacts and APK bookkeeping did not cover the new round.** `apk-b1*` added to RESULT.md and to
  `evidence/apk-md5.txt`; `docs/` described as containing the AOSP copies; the harness changes below
  documented; `REVIEW-B1.md` listed.

**Harness**

* **D29 — `run_case.sh` hid the reason for a launch crash and recorded no target SDK.** A case that died
  in `onStartCommand` read `result=PROCESS_GONE_AT_LAUNCH` with nothing to say why, and `case.txt` had no
  target-SDK field even though the runs are compared across ts37/ts34/ts33 builds. Fix: it now writes
  `fatal_exception_at_launch=<outer>;<cause>` and `target_sdk=`; verified by
  `evidence/review-b1-harness-smoke-a31-bad-name/case.txt`
  (`java.lang.RuntimeException;java.lang.IllegalArgumentException`) and
  `…-a31-combo-datasync/case.txt` (`target_sdk=37`). `bash -n` clean; RESULT.md's case rows for the
  pre-existing evidence keep their original `case.txt` format.
* **D30 — `queue.sh` could not re-run a set without destroying the previous round's evidence, and did not
  read the config keys back.** A1's re-run overwrote nothing only because it used its own prefix; a second
  round would have clobbered `review-a1-*`. Fix: `CASE_TAG=<tag>` re-labels the output dirs (this whole
  round ran with `CASE_TAG=review-b1`), and every `device_config put` is now followed by a `get`, and
  every `delete` by a `get` whose `null` is stored in `timeout_knob.txt`. End-to-end verified on 34, 37
  and 31 (`logs/b1-queue34.log`, `logs/b1-queue37.log`, `logs/b1-run31.log`).

## 5. Agreement with the recommendation

Agree, with round A1's three amendments, which B1 re-verified rather than restated:

1. **Explicit type at runtime** — `mediaProcessing` on `SDK_INT >= 35`, else `dataSync`. Re-run on 34
   (fatal to the feature without it), 37 (works, but only because 35+ knows the bit), 29/31 (works, no
   policy). B1 adds the platform-source reason the Android 14 behaviour is what it is (D20) and the
   per-level crash names (D22).
2. **Time limits only from Android 15** — re-run with the adb knobs on 34 (ignored — the keys do not exist on
   Android 14 at all) and 37 (timeouts at 61.0–61.7 s after HOME, `stopSelf()` inside the grace
   period, no ANR/crash), plus the exhaustion→foreground-reset case. The recommendation to stop the
   service when the export ends is the right advice regardless of which doc reading you prefer, because
   the platform counts only background time and resets on user interaction (D23).
3. **Two Play Console declarations + wake-lock guidance** — quotes now verbatim with URLs, including the
   "check whether WorkManager/Media3 already holds a wake lock" branch (D26, D27).

Why agree at all: the same byte-identical APK ran a 660 s export, started from the foreground and
backgrounded, on 29, 31, 34 and 37 (B1 re-ran 34 and 37: `types=00000001` and `types=0x00002000` in
`dumpsys_bg.txt` with the launcher resumed); `mediaProcessing` is the documented type for conversion work
and is prohibited on 14, so 35 is the right threshold; `dataSync` covers "Import or export operations"
and "Local file processing" and has no time limit anywhere the recommendation uses it. The alternatives
stay worse: `shortService` dies at 180 s on 34/37 (three runs), an untyped service throws for any
targetSdk ≥ 34, `specialUse` works everywhere but needs a reviewed free-form justification, and
WorkManager long workers run inside a foreground service anyway.

## 6. Open items

* API 30, 32, 33 untested (no such AVDs in this environment); expected to behave like 29/31 for types.
* The 24-hour window itself and the documented `ForegroundServiceStartNotAllowedException` for a
  **background** start against an exhausted budget: not testable without a background start (the test app
  starts from the foreground by design) and with a 6 h budget.
* `ServiceCompat.startForeground()` not exercised (RESULT.md says so).
* Everything in RESULT.md's "What the emulator cannot show" list — OEM process killers, thermal
  throttling, real Doze, notification-shade UX, and the physical-phone wake-lock check.
* `apk/`→`apk2/` "rebuilds byte-for-byte" can no longer be re-verified (that pre-review source was
  overwritten). B1's identical rebuild of `app/` → `apk-a1` supports the claim's method but does not
  re-prove that historical pair; RESULT.md's wording is a statement about the past, and it is left as is.
* The `device_config delete` message discrepancy noted in D24 (cosmetic; both rounds ended with the key absent).

## 7. Environment state after this round

Every emulator this round started — `Pixel_6_API_29`, `Pixel_6_API_31`, `Pixel_6_API_34`,
`Pixel_6_API_37`, never more than two at a time — is stopped, and `adb devices` is empty; no `run_case.sh` or
`queue.sh` is running. `com.example.fgslab` is absent from every AVD booted: uninstalled from 29, 31 and
37 in this round's final pass (`pm list packages` empty afterwards), and on 34 the queue's own closing
uninstall had already removed it. Every `device_config` key this round touched in namespace
`activity_manager` reads back `null` on both 34 and 37 — `data_sync_fgs_timeout_duration`,
`media_processing_fgs_timeout_duration` and, checked for good measure, `short_fgs_timeout_duration`
(transcripts in the `timeout_knob.txt` files and `review-b1-a37-datasync-exhaust-reset/knob.txt`). No AVD
was deleted, renamed or recreated; `Pixel_10_Pro_XL` was never started or touched. Nothing was pushed to
GitHub, posted to the issue, or committed anywhere.

**Verdict: clean — recommendation and its three amendments confirmed; 12 defects found and fixed in this round (D19–D30), no findings left open.**
