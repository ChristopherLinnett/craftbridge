# Review round A3 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: Claude, round A3, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md` and
`REVIEW-B2.md` as handed over after round B2, the earlier review files, `app/`, every APK set,
`evidence/`, `docs/`, `logs/`, the scripts. Method as before: new evidence only under
`CASE_TAG=review-a3`, earlier evidence untouched, a re-run that agrees with earlier evidence apart from
instance values is not a defect and does not change RESULT.md. Since round A2 the owner approved
repairing `Pixel_6_API_29`; it was wiped and boots again, so this round re-ran the API 29 cases.

**Verdict: 4 defects found and fixed (D49–D52), all in bookkeeping and the harness; the recommendation, its three amendments and every platform claim stand. Not clean.**

## 1. B2 claims and fixes checked

| B2 claim / fix | How checked | Result |
|---|---|---|
| D45: six D37 figures 61.05 / 61.07 / 61.07 / 61.07 / 61.08 / 61.11 s, range 61.05–61.11 s | HOME = the `Starting home with component specified` line, timeout = `FGS (<type>) timed out`, both from each run's `logcat_full.txt` | true (61.047, 61.066, 61.067, 61.065, 61.084, 61.109 s). My A2 figures had taken the first of several matching HOME-ish lines in some runs; B2 is right |
| D46: HOME → left TOP 1.04–1.10 s; `lastTopTime` → timeout 60.02 s in all seven A2 runs | recomputed from `processes_bg.txt` + the two log lines of the seven runs | true (1.039–1.095 s; 60.016–60.023 s) |
| D47: the "in addition to requesting the `FOREGROUND_SERVICE` permission" sentence is on the service-types page, not the Android 14 page | `docs/service-types.txt` line 42 vs `docs/v14-required.txt` | true |
| D48: re-running a label kept stale background snapshots; fixed by deleting the three files at case start | read `run_case.sh`; re-ran a label twice on 31 (§2) | true but incomplete: a run that dies at launch writes no `dumpsys_early`/`dumpsys_late`, so the first run's copies survived the B2 fix → D50 |
| B2's queues ran `apk2/` (queue default), not `apk-b1/` | `apk_md5` of all 34 `review-b2-*` dirs | 27 dirs `0cea776e…` (apk2 combo), 2 `c84df81a…` (apk2 full), 3 `3d45b867…` (`review-b2-b1apk-*`), 2 knob-transcript dirs without `case.txt`. **RESULT.md cites no `review-b2-*` directory** (grep), so no claim rests on the wrong APK. The default that caused it is a harness hazard → D51 |
| "the two builds differ in exactly one exercised path, bad-type-name" | `apk2` vs `apk-b1` source history (A1 D3/D16) and `review-b2-a34-combo-bad-type-name` | true for outcomes; `apk2` also logs the two-argument timeout as `FGS_ON_TIMEOUT fgsType=<startId> timeoutSource=<fgsType>` (A1 D3), so B2's "every outcome … reproduces from apk2 exactly as A2 stated" holds for outcomes, not log text. B2's "dies seconds later" for apk2's bad-type crash is 65–103 ms in its own logs (34, 31, 37) |
| §7: 34 keys incl. `short_service_fgs_timeout_duration` read back `null` | key names in `docs/aosp-android14-release-ActivityManagerConstants.java` | that key does not exist; the platform key is `short_fgs_timeout_duration`. Nothing in any round ever set it, so nothing is left set; noted, no fix possible in B2's file |
| §7: "never more than two were up at a time" | same paragraph | contradicted by its own parenthesis (a transient third emulator, pid 81980); noted |
| RESULT.md header and end state | read | still "corrected by review rounds A1, B1 and A2", end state pointing at `REVIEW-A2.md`, and "`Pixel_6_API_29` no longer boots" — stale after B2 and the owner's repair → D49 |
| 33 checksums in `evidence/apk-md5.txt` | recomputed | all match |

## 2. Cases re-run (review A3)

Emulators started this round, tracked by pid (`logs/a3-pids.txt`): `Pixel_6_API_29` 89600 and
`Pixel_6_API_31` 89601 (14:38:39), then `Pixel_6_API_34` 13291 and `Pixel_6_API_37` 13292 (14:58:10,
state check only). Two at a time. All cases ran `apk-b1/` (combo `3d45b867…`), passed to `queue.sh`
explicitly; `queue.sh <serial> <api> amendments apk-b1` with `CASE_TAG=review-a3`, then single
`run_case.sh` calls for the harness checks. 17 new directories, all `review-a3-*`.

`Pixel_6_API_29` after the repair: `sdk=29`, 174 packages, 0 `exactly one installer` lines, no test app.

| Case | Device | Result | Against earlier evidence |
|---|---|---|---|
| `review-a3-a29-combo-auto-long` | 29 | `STARTFOREGROUND_OK requested=auto typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=420224`; `dumpsys_bg.txt` `isForeground=true`, launcher resumed, process state FGS (`curProcState=5`) | agrees with `a29-combo-final` (0cea…); first 660 s run of the reviewed build on 29 |
| `review-a3-a29-combo-none` | 29 | ✅ `typeBit=0` → `EXPORT_DONE` | agrees with `review-b1-a29-combo-none` |
| `review-a3-a29-combo-short-notdeclared` | 29 | `IllegalArgumentException … not a subset … 0x00002001` → `android.app.RemoteServiceException` +84 ms | agrees with `review-b1-a29-combo-short-notdeclared` |
| `review-a3-a29-combo-mediaproc`, `-datasync` | 29 | ✅ `typeBit=8192`; ✅ `typeBit=1` | agree (the mediaproc `case.txt` carries a foreign crash field → D52, `NOTE-review-a3.txt`) |
| `review-a3-a29-combo-bad-type-name` | 29 | `am start -W` bounded, `APP_CRASHED`, `RuntimeException;IllegalArgumentException` | agrees with A1's 29 run |
| `review-a3-a31-combo-auto-long` | 31 | `typeBit=1` → `EXPORT_DONE elapsedS=660 fileKiB=417792`, bg `isForeground=true`, launcher resumed | agrees with `a31-combo-final`; first 660 s run of the reviewed build on 31 |
| `review-a3-a31-combo-{none,mediaproc,datasync}` | 31 | ✅ ✅ ✅ | agree |
| `review-a3-a31-combo-short-notdeclared` | 31 | not-a-subset → `ForegroundServiceDidNotStartInTimeException` +136 ms | agrees |
| `review-a3-a31-combo-bad-type-name` | 31 | `APP_CRASHED`, `RuntimeException;IllegalArgumentException` | agrees |
| `review-a3-harness-a31-relabel-b2` | 31 | B2's `run_case.sh` (copy `logs/a3-run_case-as-of-b2.sh`): run 1 dataSync, run 2 shortService into the same label → `PROCESS_GONE_AT_LAUNCH`, yet run 1's `dumpsys_early.txt`/`dumpsys_late.txt` (`isForeground=true`) are still there | D50 |
| `review-a3-harness-a31-relabel-fixed`, `-fixed2` | 31 | fixed `run_case.sh`, same two runs: after a launch death only `am_start`, `case`, `install`, `logcat_full` remain | D50 fix verified |
| `review-a3-harness-a29-crash-then-ok-{1,2}` | 29 | fixed `run_case.sh`: undeclared-type crash, then mediaProcessing; the second records no crash field | D52 fix (the leak did not recur this time; the fixed function was also run on the saved logs of the leaking case, §4) |

Re-runs that agree changed RESULT.md in one place only: the reviewed build's 660 s export on 29 and 31
is new coverage (before, it had 45 s runs there), so the verdict sentence and the one-APK table now cite
it. That is coverage, not a defect.

## 3. Documentation and source read

No new documentation claims this round. Re-read for B2's items: `docs/service-types.txt` (line 42, the
`FOREGROUND_SERVICE` sentence) and `docs/v14-required.txt` (no such sentence); key names in
`docs/aosp-android14-release-ActivityManagerConstants.java` and the android15 copy
(`short_fgs_timeout_duration`; no `short_service_fgs_timeout_duration`).

## 4. Defects found and fixed

Files changed this round: `RESULT.md`, `run_case.sh`, `queue.sh`; new `REVIEW-A3.md`; new
`evidence/review-a3-*` (17 directories, one with `NOTE-review-a3.txt`); new `logs/a3-*` (queue and
emulator logs, `a3-pids.txt`, `a3-final-state.txt`, the harness checks, and
`a3-run_case-as-of-b2.sh`, the B2 harness kept to reproduce D50). No app source change; no earlier
evidence or review file changed.

* **D49 — RESULT.md's bookkeeping was stale after round B2.** The header still read "corrected by review
  rounds A1, B1 and A2" though B2 changed RESULT.md; the artifacts list had no `REVIEW-B2.md` and no
  `review-b2-*` labels; the end-state line pointed at `REVIEW-A2.md` and said `Pixel_6_API_29` "no
  longer boots", which the owner's repair has since made untrue. Fix: header, end state, artifacts and
  label list updated.
* **D50 — B2's D48 fix was incomplete.** It deletes the three background-snapshot files, but a run that
  dies at launch writes no `dumpsys_early.txt`/`dumpsys_late.txt` either, so a re-used label kept the
  previous run's copies: `review-a3-harness-a31-relabel-b2` ends `PROCESS_GONE_AT_LAUNCH` next to a
  `dumpsys_early.txt` that shows `isForeground=true` from the earlier run. Fix: `run_case.sh` removes
  every file it writes (`am_start`, `install`, `logcat_full`, `dumpsys_early`, `dumpsys_bg`,
  `resumed_bg`, `processes_bg`, `dumpsys_late`) at case start; `timeout_knob.txt` belongs to the queue
  and is kept. Verified: `review-a3-harness-a31-relabel-fixed2`.
* **D51 — `queue.sh` silently defaulted to the superseded `apk2/` build, and RESULT.md did not say which
  build each round's evidence ran.** That default is how 27 of B2's 34 directories came to run `apk2/`
  (`0cea776e…`) while B2 believed it re-ran `apk-b1/`. None of them is cited by RESULT.md, so no claim
  is affected. Fix: the APK directory is a required argument (`queue.sh` aborts without a directory that
  holds `fgslab-combo-ts37.apk`; checked both failure modes), and RESULT.md's APK-set list names the
  build behind `review-b1-*`, `review-a2-*`, `review-a3-*`, `review-b2-b1apk-*` and the other
  `review-b2-*` cases.
* **D52 — the crash fields were not tied to the run's own process.** A2's `record_end_fields` (D44) took
  the first crash of the package, and on API 29 `logcat -b all -c` leaves the previous case's crash in
  the buffer: `review-a3-a29-combo-mediaproc` ended `EXPORT_DONE` with
  `fatal_exception=android.app.RemoteServiceException` copied from pid 3717 (the preceding
  `short-notdeclared` case); its own pid 3807 never crashed. `target_sdk=` was unscoped the same way.
  Fix: both fields come from this run's pid (from `pidof`, or for a launch death from the run's last
  `ACTIVITY_STARTED_FGS_FROM_FOREGROUND` line); the `Process: <pkg>, PID: <pid>` match is exact.
  Checked on the saved logs: the mediaproc case now yields no crash field, while
  `review-a3-a29-combo-short-notdeclared`, `review-a3-a29-combo-bad-type-name`,
  `review-a2-a34-combo-short-notdeclared` and `review-a2-a31-combo-bad-type-name` yield their own
  crashes. A search of all `case.txt` files finds no other case that ended normally with a borrowed
  crash field (the `STARTFOREGROUND_FAIL` + crash pairs in `review-b2-*-bad-type-name` are real: `apk2/`
  catches the name error and then dies of `ForegroundServiceDidNotStartInTimeException`).

## 5. Agreement with the recommendation

Agree, unchanged: declare `mediaProcessing|dataSync` with `FOREGROUND_SERVICE`,
`FOREGROUND_SERVICE_MEDIA_PROCESSING` and `FOREGROUND_SERVICE_DATA_SYNC`, and pass `mediaProcessing`
when `SDK_INT >= 35`, else `dataSync`, with round A1's three amendments. The API 29 re-run closes the gap
left since B1: on the repaired AVD the reviewed build exported for 660 s in the background as `dataSync`,
a start without a type works (no type policy on Android 10), and an undeclared type kills the app with
`RemoteServiceException`, as RESULT.md says. The four defects are bookkeeping and harness defects; none
touches the recommendation or a platform claim.

## 6. Open items

* Carried over, unchanged: API 30/32/33 untested; the full 24 h window; a background start against an
  exhausted budget; `ServiceCompat.startForeground()`; everything in RESULT.md's "What the emulator
  cannot show" list.
* REVIEW-B2.md's own inaccuracies (the non-existent `short_service_fgs_timeout_duration` key, "never
  more than two" next to a third emulator, "dies seconds later") are recorded in §1; earlier review
  files stay unchanged by rule. The real `short_fgs_timeout_duration` key reads `null` on all four AVDs (§7).

## 7. Environment state after this round

Checked at the end with `ps ax` (no `qemu-system`, `emulator -avd` or emulator `crashpad_handler`
process) and `adb devices` (empty). Every emulator started this round is gone: 89600
(`Pixel_6_API_29`), 89601 (`Pixel_6_API_31`), 13291 (`Pixel_6_API_34`), 13292 (`Pixel_6_API_37`), each
stopped with `adb emu kill` and confirmed by pid. Read back before each stop (`logs/a3-final-state.txt`):
`com.example.fgslab` absent on all four (uninstalled on 29 and 31, already absent on 34 and 37);
`data_sync_fgs_timeout_duration`, `media_processing_fgs_timeout_duration` and
`short_fgs_timeout_duration` `null` on all four; `get_sync_disabled_for_tests` `none` on 34 and 37, the
command absent on 29 and 31 (nothing set there). This round set no `device_config` key. No AVD deleted,
renamed, recreated or wiped by me; `Pixel_10_Pro_XL` not started. No Gradle build this round. Nothing was
committed, pushed or posted.

**Verdict: 4 defects found and fixed (D49–D52), all in bookkeeping and the harness; the recommendation, its three amendments and every platform claim stand. Not clean.**
