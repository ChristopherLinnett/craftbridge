# Review round B3 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: build side, round B3, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md` and
`REVIEW-A3.md` as handed over after round A3, the earlier review files, `app/`, every APK set,
`evidence/`, `docs/`, `logs/`, the scripts. Method as before: new evidence only under
`CASE_TAG=review-b3`, earlier evidence untouched, a re-run that agrees with earlier evidence apart from
instance values is not a defect and does not change RESULT.md; the APK directory was passed to
`queue.sh` explicitly, as A3's D51 fix now requires.

**Verdict: 1 defect found and fixed (D53), a citation-fidelity error in RESULT.md's Android 14 fallback paragraph; the recommendation, its three amendments and every platform claim stand. Not clean.**

## 1. A3 claims and fixes checked

| A3 claim / fix | How checked | Result |
|---|---|---|
| D49: RESULT.md header, artifacts list, end-state pointer and API 29 wording updated | read RESULT.md lines 1–6, 44–65, 519–530 | true: "corrected by review rounds A1, B1, A2, B2 and A3", `REVIEW-B2.md` and `REVIEW-A3.md` in the artifacts list, end state points at `REVIEW-A3.md` §7, API 29 described as repaired and re-run |
| D50: `run_case.sh` now removes every file it writes at case start | read the fix; re-ran one label twice on 31 (§2, `review-b3-harness-a31-relabel-b3`): run 1 dataSync `EXPORT_DONE`, run 2 same label shortService `PROCESS_GONE_AT_LAUNCH` | true: after the launch death the directory holds only `am_start.txt`, `case.txt`, `install.txt`, `logcat_full.txt` — nothing from run 1 survives |
| D51: `queue.sh` aborts without a valid APK directory; RESULT.md names the build behind every round's cases | re-ran both abort modes (`queue.sh` → rc=1, bad dir → rc=1, clear message); recomputed the md5 of all 33 files in `evidence/apk-md5.txt` (all match); compared the `apk_md5=` of **every** `case.txt` against the per-round build list at RESULT.md lines 85–87 | true: zero violations across all ~100 case directories (`review-b1-*`/`review-a2-*`/`review-a3-*`/`review-b2-b1apk-*` all `3d45b867…`, the other `review-b2-*` all `0cea776e…`/`c84df81a…`); RESULT.md cites no `review-b2-*` case (grep), so nothing rests on the superseded build |
| D52: crash fields now come from the run's own pid only | re-ran the extractor over the saved logs (faithful replay): `review-a3-a29-combo-mediaproc` → no field, `review-a3-a29-combo-short-notdeclared` → `android.app.RemoteServiceException`, `review-a3-a29-combo-bad-type-name` → `RuntimeException;IllegalArgumentException`, `review-a2-a34-combo-short-notdeclared` and `review-a2-a31-combo-bad-type-name` → their own classes; live pair on 29 (§2, `review-b3-harness-a29-crash-then-ok-{1,2}`) | true: the relabel note + `NOTE-review-a3.txt` are in place, and live: run 1 (undeclared type) crashes and records its exception, run 2 (mediaProcessing, `EXPORT_DONE`) records no crash field although run 1's crash was still in the buffer. The same holds at queue level: my `review-b3-a29-combo-mediaproc` (running right after a crashing case, whose log really contains a foreign `FATAL EXCEPTION`) has no `fatal_exception` field |
| §2 table: all 17 `review-a3-*` re-runs agree with earlier evidence; every one ran `apk-b1/` explicitly | my own 12 re-runs of the same amendment queues on 29 and 31 (§2); `apk_md5=` and `device_sdk=` of all 17 `review-a3-*` `case.txt` | agree: 17/17 carry `3d45b867…` and an SDK matching the label; every result, type bit, exception class and quote my re-runs produced matches A3's table apart from instance values (pids, timestamps, `fileKiB` — 29: 417,280 KiB vs 420,224; 31: 416,128 KiB vs 417,792, both inside the spread of the earlier 660 s runs 413,632–420,224) |
| §2: A3's stated crash deltas (+84 ms on 29, +136 ms on 31) | the two `STARTFOREGROUND_FAIL` → `FATAL EXCEPTION` pairs in A3's own logs | true (14:39:50.262→.346 = 84 ms; 14:39:51.334→.470 = 136 ms) |
| §2: `Pixel_6_API_29` after repair: 174 packages, no test app | `pm list packages \| wc -l` on the AVD this round | 174, test app absent before install |
| §7: end state (all four emulators stopped; package absent; three timeout keys `null`; `get_sync_disabled_for_tests` `none` on 34/37, absent on 29/31) | `logs/a3-pids.txt`, `logs/a3-final-state.txt` re-read; my own read-back on 29 and 31 at the end of this round (§7) | consistent everywhere I could re-measure |
| §6: the three recorded REVIEW-B2.md inaccuracies (nonexistent `short_service_fgs_timeout_duration`, "never more than two", "dies seconds later") | key names in both AOSP `ActivityManagerConstants` copies; all eight B2 `timeout_knob.txt`/`knob.txt` transcripts (only the two real keys were ever put, all deleted and read back `null`); B2's own crash timestamps | all three recordings stand; earlier review files stay unchanged by rule, so nothing further to fix |
| RESULT.md: all case citations | every `evidence/...` directory cited in RESULT.md exists (95 tokens checked); each of the 49 build-side cases has `install_rc=0` + `Success`; label-vs-device-SDK cross-check over all directories; the cited numbers re-grepped: 660 s `fileKiB` (415232/417280/414528/417792/420224/413632/416512/419392/408256/409664/413440/421504/38208), timeout bands (61.047–61.109 → "61.05–61.11"; seven 60.016–60.023 → "60.02"; 1.039–1.095 → "1.04–1.10"), the three 180.0 s shortService anchors, the 55–192 ms + 649 ms crash band, per-API exception spellings, `types=` fields, the exhaustion pair, the B1 re-run table, the one-APK table, A1's knob60 timestamps (`09:24:01.849`/`09:25:11.030`, HOME 09:24:10, 61.03 s/69.18 s), `FGS_INTRODUCE_TIME_LIMITS` absent from 34's `dumpsys platform_compat` and present with `enableSinceTargetSdk=35` on 37, the noperm APK's badging (`FOREGROUND_SERVICE` + `POST_NOTIFICATIONS` only, `foregroundServiceType=0x40002001`), the void directory and its `VOID-review-b1.txt` | no claim unsupported → but one source **quote** is not verbatim → D53 |
| Environment table vs AVDs | `config.ini` of all four AVDs | all four system-image lines match |

## 2. Cases re-run (review B3)

Emulators started this round, tracked by pid (`logs/b3-pids.txt`): `Pixel_6_API_29` 16451 (port 5554)
and `Pixel_6_API_31` 16452 (port 5556), both at 15:07:53, never a third, both stopped by me (§7).
`Pixel_6_API_34`/`37` were not needed and not started; `Pixel_10_Pro_XL` untouched.
All cases ran `apk-b1/` (combo `3d45b867…`) passed to `queue.sh` explicitly;
`queue.sh <serial> <api> amendments apk-b1` with `CASE_TAG=review-b3`, then single `run_case.sh`
calls for the harness checks. 15 new directories, all `review-b3-*`.

| Case | Device | Result | Against earlier evidence |
|---|---|---|---|
| `review-b3-a29-combo-auto-long` | 29 | `STARTFOREGROUND_OK requested=auto typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=417280`; bg dumpsys `isForeground=true`, launcher resumed (`curProcState=5`), `dumpsys_late.txt` empty | agrees with `review-a3-a29-combo-auto-long` (420224) — instance values only; RESULT.md's A3 verdict citation re-confirmed in A3's own log |
| `review-b3-a29-combo-none` | 29 | ✅ `typeBit=0` → `EXPORT_DONE`, bg `isForeground=true` with launcher resumed | agrees |
| `review-b3-a29-combo-short-notdeclared` | 29 | `IllegalArgumentException … not a subset … 0x00002001` → `PROCESS_GONE_AT_LAUNCH`, `fatal_exception_at_launch=android.app.RemoteServiceException` (+104 ms) | agrees (+84 ms in A3; my earlier whole-set band 55–192 ms covers both) |
| `review-b3-a29-combo-mediaproc`, `-datasync` | 29 | ✅ `typeBit=8192`; ✅ `typeBit=1`; mediaproc carries **no** `fatal_exception` field although the buffer still held the previous case's crash | agrees, and confirms D52 end-to-end in the very queue position where A3 caught the leak |
| `review-b3-a29-combo-bad-type-name` | 29 | `am start -W` bounded, `APP_CRASHED`, `RuntimeException;IllegalArgumentException` | agrees |
| `review-b3-a31-combo-auto-long` | 31 | `typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=416128`, bg `isForeground=true`, launcher resumed, nothing in `dumpsys_late.txt` | agrees with A3 (417792); instance values only |
| `review-b3-a31-combo-{none,mediaproc,datasync}` | 31 | ✅ ✅ ✅ (`typeBit=0/8192/1`) | agree |
| `review-b3-a31-combo-short-notdeclared` | 31 | not-a-subset → `android.app.ForegroundServiceDidNotStartInTimeException` (+166 ms) | agrees (+136 ms in A3) |
| `review-b3-a31-combo-bad-type-name` | 31 | `APP_CRASHED`, `RuntimeException;IllegalArgumentException` | agrees |
| `review-b3-harness-a31-relabel-b3` | 31 | same label twice with the fixed script: dataSync run then shortService launch death → only the four files of run 2 remain | D50 fix verified on a fresh label |
| `review-b3-harness-a29-crash-then-ok-{1,2}` | 29 | crash (undeclared type, `RemoteServiceException` recorded), then mediaProcessing `EXPORT_DONE` with no crash field | D52 fix verified live |

Queue transcripts: `logs/b3-queue29.log`, `logs/b3-queue31.log` (both `QUEUE_amendments_* DONE`,
uninstall `Success` at the end of each).

## 3. Documentation and source read

Re-read this round, beyond what is already in §1: RESULT.md end to end; the two fenced AOSP blocks
against their local copies — the android15 `onFgsTimeout` block (lines 375–381) matches
`docs/aosp-android15-release-ActiveServices.java` (3900–3909) verbatim including its `…` elision, the
android14 `defaultFgsTypes` block did **not** → D53; the prose citations around them
(`mForegroundServiceTypePolicies.get(type)`, "Unknown type, fallback to the defaultToType",
"FGS_TYPE_POLICY_CHECK_DISABLED", `setServiceForegroundInnerLocked`, the reset comment "…currently in
the TOP state or was in the TOP state after the time limit was exhausted previously", the concatenated
"Time limit already exhausted for foreground service type …" thrown as
`ForegroundServiceStartNotAllowedException`) all check out against the saved files — the exception
branch is flag-gated (`enableFgsTimeoutCrashBehavior()`) in the source, which RESULT.md neither states
nor relies on (its claims — reset needs a foreground start, the exception branch needs a background
start this app does not make — hold either way), so it is recorded here, not corrected; the android15
`6 * 60 * 60_000 // 6 hours` defaults for both budget keys; the export loop constants in
`app/fgslab/src/main/java/com/example/fgslab/ExportService.kt` (40 ms sqrt spin, 65536-byte append,
60 ms sleep, 5 s notification, 60 s TICK, both `onTimeout` overrides) against RESULT.md's test-app
paragraph. The earlier rounds' documentation quotes (service-types, timeout page, Service reference,
ServiceInfo, awake page, v14-required, play console, datasync-migration, and the live re-check of the
troubleshooting page) were confirmed in previous rounds and were not contradicted by anything read now.

## 4. Defects found and fixed

Files changed this round: `RESULT.md` (two lines, D53); new `REVIEW-B3.md`; new
`evidence/review-b3-*` (15 directories); new `logs/b3-*` (queue and emulator logs, `b3-pids.txt`,
`b3-final-state.txt`). No app source change; no APK rebuild (nothing in the evidence required one);
no earlier evidence or review file changed.

* **D53 — RESULT.md's Android 14 fallback paragraph did not reproduce the sources it cites.**
  The fenced `int defaultFgsTypes = …` block introduced as what "the platform source … states", with
  the link and local copy named, dropped the three `ServiceInfo.` qualifiers the quoted code carries
  and reflowed two lines into one, so it matched neither the local copy (line 2315) nor upstream; no
  static import would license the unqualified form (the constant occurs qualified in both of its
  appearances in the file). In the same paragraph the `fileManagement` comment was rendered
  `"will default to 'none'"` while the comment reads `will default to "none"`. Fix: the block now
  reproduces the local copy verbatim (modulo the block's own left indentation — `diff` of the
  dedented lines is empty) and the prose quote is the exact comment fragment. Nothing in the
  reasoning changes: the fallback rule, the two runs it explains and every measured number stand.

## 5. Agreement with the recommendation

Agree, unchanged: declare `mediaProcessing|dataSync` with `FOREGROUND_SERVICE`,
`FOREGROUND_SERVICE_MEDIA_PROCESSING` and `FOREGROUND_SERVICE_DATA_SYNC`, and pass `mediaProcessing`
when `SDK_INT >= 35`, else `dataSync`, with round A1's three amendments. My re-runs on the repaired
API 29 AVD and on 31 — the amendment matrix and both 660 s background exports — reproduce A3's
results apart from instance values. The one defect this round is a quotation error in RESULT.md;
nothing found this round touches the recommendation or a platform claim.

## 6. Open items

* Carried over, unchanged: API 30/32/33 untested; the full 24 h window; a background start against an
  exhausted budget (`enableFgsTimeoutCrashBehavior()` makes even the exception conditional — §3);
  `ServiceCompat.startForeground()`; everything in RESULT.md's "What the emulator cannot show" list.
* REVIEW-B2.md's own inaccuracies remain as recorded by A3 (§6 there); they are confirmed accurate
  recordings (§1), and earlier review files stay unchanged by rule.

## 7. Environment state after this round

Checked at the end with `ps ax` (no `qemu-system`, no `emulator -avd`, no emulator `crashpad_handler`;
the only `crashpad_handler` processes belong to VS Code, Chrome and Claude) and `adb devices` (empty).
Every emulator started this round is gone: 16451 (`Pixel_6_API_29`), 16452 (`Pixel_6_API_31`), each
stopped with `adb emu kill` and confirmed by pid (`logs/b3-pids.txt`). Read back before the stops
(`logs/b3-final-state.txt`): `com.example.fgslab` uninstalled on both (`Success`, `pm list packages`
empty); `data_sync_fgs_timeout_duration`, `media_processing_fgs_timeout_duration` and
`short_fgs_timeout_duration` `null` on both; `get_sync_disabled_for_tests` absent on both (the command
does not exist on 29/31; this round set no `device_config` key at all — the amendment queues on
API < 34 contain no knob case, and nothing needs resetting). `Pixel_6_API_34`/`Pixel_6_API_37` were
not started this round; their state is A3 §7 and I re-read its saved transcript (§1). No AVD deleted,
renamed, recreated or wiped by me; `Pixel_10_Pro_XL` not started. No Gradle build this round. Nothing
was committed, pushed or posted.

**Verdict: 1 defect found and fixed (D53), a citation-fidelity error in RESULT.md's Android 14 fallback paragraph; the recommendation, its three amendments and every platform claim stand. Not clean.**
