# Review round B2 — CraftBridge V7 (#30): foreground-service type for a long export

Reviewer: Claude, round B2, working alone in `/tmp/craftbridge-v7`. Inputs: `RESULT.md`, `REVIEW-A2.md`,
`REVIEW-B1.md`, `REVIEW-A1.md` as handed over after round A2, `app/`, every APK set, `evidence/`, `docs/`,
`logs/`, `run_case.sh`, `queue.sh`, `summarize.sh`. Method, as set for this round: every re-run went into
new evidence directories (`CASE_TAG=review-b2`, plus `review-b2-guarded` for the fixed-harness re-runs);
no earlier evidence was changed; a re-run that agrees with earlier evidence apart from instance values
(pids, timestamps, durations within their observed spread) is not a defect and did not change RESULT.md.
RESULT.md was changed only where a claim, number or citation was wrong, missing or unsupported.

**Verdict: 4 defects found and fixed (D45–D48); the recommendation and its amendments stand unchanged.**

## 1. A2 claims and fixes checked

| A2 claim / fix | How checked | Result |
|---|---|---|
| All 33 md5s in `evidence/apk-md5.txt`; RESULT's quoted combo/full/none/noperm checksums | recomputed `md5` of every file named there and of the cited sets | all match; `apk2` combo `0cea776e…`, `apk-a1` = `apk-b1` combo `3d45b867…` as RESULT states |
| §2: 31 fresh `review-a2*` case dirs; every `case.txt` records `apk_md5`, `device_sdk`, `pid`, `home_sent_device`, `target_sdk` | field loop over all `review-a2*/case.txt` | directories and counts agree; `home_sent_device` is absent in the three `review-a2-a{31,34,37}-combo-short-notdeclared` cases (the app dies at launch before the HOME step, and my own fresh re-runs behave the same) — A2's "every" overclaims slightly; RESULT's procedure text (line "device time … from review A2 on") describes the cases that reach HOME and stays true, so RESULT unchanged |
| §2 table: outcomes and numbers per re-run (fileKiB of the 660 s runs, timeout offsets, `types=` values) | recomputed from every cited case's `logcat_full.txt` / `dumpsys_*.txt` | all reproduce: 660 s fileKiB 415232/417280/413568/414528/413632/416512/419392/419264/408256/409664/413440/414464 as cited; `dumpsys_early` `types=40002001` (a34-full-none), `0x40002001` (a37), `00000001` dataSync-bg, `0x00002000` auto-bg; `dumpsys_late` `(nothing)` after every 660 s run; `isForeground=true` + launcher resumed in `dumpsys_bg`/`resumed_bg` |
| "55–192 ms in eight runs, 649 ms in `review-a2-a31-combo-short-notdeclared`" (D41 fix) | `STARTFOREGROUND_FAIL`→`FATAL` deltas per case | reproduces exactly: 76/147/186/98 (B1 29/31/34/37), 68/55 (build-side), 192/79 (A2 34/37), 649 (A2 31); fresh B2 runs: +89 ms (34), +60 ms (37) — same classes, all inside the stated 55–192 ms spread |
| `fatal_exception=` fields (D44) | `case.txt` of the three `review-a2-*-combo-bad-type-name` and the notdeclared cases | `java.lang.RuntimeException;java.lang.IllegalArgumentException` on all three as claimed; notdeclared cases record the launch-path pair (first item is the exception class) |
| D37 six-run figures and the `lastTopTime` resolution | re-measured all six runs and all seven D38 runs from device log lines + `processes_bg.txt` | lastTopTime→timeout = 60.02 s in all seven ✓ and HOME→timeout in 61.05–61.11 s — but three of the six printed values were +0.01 s high → D45; the HOME→TOP-exit spread is 1.04–1.10 s, not 1.04–1.06 → D46 |
| D38/D39 source claims (android15 `ActiveServices` lines 2437–2466, `getNextFgsStopTime` 3773, `onFgsTimeout` 3887; android14 comment 2311–2314, policy fallback 1347–1356) | re-read `docs/aosp-android15-release-ActiveServices.java`, `docs/aosp-android14-release-*.java` at those lines | every line reference and quote lands where A2 says (`fgsTypeInfo.reset()` at 2449/2464, `// currently in the TOP state or was in the TOP state after` at 2447, `private long getNextFgsStopTime` at 3773, `void onFgsTimeout` at 3887, `// Unknown type, fallback to the defaultToType` inside 1347–1356) |
| D42 fix: suspend flag sync around every knob, read key back before deleting, restore `none` | read `queue.sh` (`knob_on`/`knob_off`); re-ran both `amendments` and `timeouts` sets on 34 and 37 with the fixed harness; transcripts `timeout_knob.txt` | works on both levels: `set_sync_disabled_for_tests until_reboot` → put → 60000 read back mid-run → delete → `null` → `none` at the end; the two unguarded A2 cases keep their `Failed to delete` + NOTE. On 31 `set_sync_disabled_for_tests` does not exist (`Invalid command`, live-checked), consistent with A2 §7 |
| "flag sync removes the local override ~7 min after boot" | A2's emulator logs: boot 12:12:30, `Boot completed in 9035 ms` on 37 → sync storm `SettingsToPropertiesMapper` + `flags_health_check ServerConfigurableFlagsReset` at 12:19:41, 26 s after the key was set | ≈7 min after boot ✓ (the mapper itself only mirrors native-scope config into system properties; the Config-table overwrite comes from the image's flag sync — exactly what "Play-services flag sync" in RESULT means) |
| D42 "against the AOSP source for those levels" | fetched and saved `packages/SettingsProvider/src/com/android/providers/settings/DeviceConfigService.java` android14-release, android15-release and android16-release (newest public branch; no android17 branch exists in the mirror) | the verb exists on all three with identical semantics: "set_sync_disabled_for_tests SYNC_DISABLED_MODE … When in one of the disabled modes this ensures that config isn't overwritten. … until_reboot: Sync is disabled until the next reboot"; mode stored via `DeviceConfig.setSyncDisabledMode` (Settings.Config). AOSP-side source + live device behavior (above) both agree; neither the android14 nor the android16 `SettingsToPropertiesMapper` references the mode — enforcement is on the flag-writing side, not the AOSP mapper, as RESULT's wording implies |
| D40 rebuild (`logs/a2-build-*.log`) | four logs checked: one `BUILD SUCCESSFUL` each, zero warning lines; A2 rebuilt bytes equal `apk-b1*` per §2 | agrees with the logs and the checksums on disk |
| Play Console quotes (point 9), wake-lock quotes (point 10), "did not stop within its timeout", "180.0 s", throughput ≈630 KiB/s | re-compared against `docs/play-console-fgs.txt`, `docs/awake.txt`, `docs/fgs-timeout.txt`; live re-check of `datasync-migration` and `troubleshooting` (not saved before; now saved as `docs/datasync-migration.txt`) | verbatim; "is available if your app targets Android 15 or higher" and "Encoding or decoding a media file" verified live on the data-transfer page ✓ |
| A2 §7 environment end state | `ps`, `adb devices`, package + `device_config` state I could observe at start | agrees with what remained: no stray qemu of mine at round start, nothing pre-set on 34/37 (all three keys `null`, sync `none`) |

## 2. Cases re-run (review B2)

All B2 cases ran `queue.sh` with `CASE_TAG=review-b2` (`amendments` and `timeouts` sets on 34 and 37,
the same two sets on 31) and — after D48 was found — the whole `timeouts` set again on 34 and 37 under
`CASE_TAG=review-b2-guarded`. One caveat, found while writing this table: those queues were started
without the queue's optional fourth argument (the APK directory, which defaults to `apk2/`), so the B2
re-runs ran the original `apk2` binaries where A2's re-runs used `apk-b1/` (A2's `case.txt` shows
`apk_md5=3d45b867…`). The two builds differ in exactly one exercised path — the *bad-type-name* case:
`apk2` catches the `IllegalArgumentException: unknown type name: bogus`, logs `STARTFOREGROUND_FAIL`
and dies seconds later of `ForegroundServiceDidNotStartInTimeException`, while `apk-a1`/`apk-b1` (A1's
fixed source, "crash by design") crash immediately with `RuntimeException←IllegalArgumentException`.
Every other B2 case passes a type name that is handled identically in both builds (the `bogus` name only
occurs in `combo-bad-type-name`), and every outcome, number and `dumpsys` value in the table below
reproduces from `apk2` exactly as A2 stated; the bad-type-name claim was verified separately — against
A2's own `review-a2-a{31,34,37}-combo-bad-type-name` evidence (all three record
`fatal_exception=java.lang.RuntimeException;java.lang.IllegalArgumentException`, `result=APP_CRASHED`)
plus fresh single-case runs with the correct `apk-b1/` binary (`review-b2-b1apk-a{31,34,37}-…`, §2 table).
The fresh apk-b1 run on 31 reproduced the same exception pair on the launch path
(`fatal_exception_at_launch=`; which of the two fields records it depends on a poll race, not behavior).
No earlier evidence directory was touched.

| case directory | device | outcome |
|---|---|---|
| `review-b2-a34-combo-short-notdeclared` | 34 | `$FGSNDSITE` crash +89 ms after `STARTFOREGROUND_FAIL` (result PROCESS_GONE_AT_LAUNCH) — A2's re-run was +192 ms, B1's +186; both inside RESULT's 55–192 ms band |
| `review-b2-a37-combo-short-notdeclared` | 37 | same class, +60 ms (A2's +79, B1's +55/98) |
| `review-b2-a3{4,7}-combo-{none,mediaproc,datasync}` | 34, 37 | 34: none+mediaproc `STARTFOREGROUND_FAIL InvalidForegroundServiceTypeException` no FATAL, dataSync `EXPORT_DONE`; 37: all three `EXPORT_DONE` — exactly the A2/B1 pattern |
| `review-b2-a31-combo-{none,mediaproc,datasync,short-notdeclared}` | 31 | accepted-and-ran for none/mediaproc/datasync (`typeBit=8192` for mediaProcessing), notdeclared crashes — as claimed |
| `review-b2-a34-combo-auto-long`, `review-b2-a37-combo-auto-long` | 34, 37 | `EXPORT_DONE elapsedS=660 fileKiB=419136` / `413952` — same throughput band (≈630 KiB/s) |
| `review-b2-guarded-a34-full-short-timeout` | 34 | `FGS_ON_TIMEOUT` at SF→TO 180.00 s (`shortService` budget) |
| `review-b2-guarded-a34-combo-datasync-knob60` | 34 | key 60000 put, read back in force twice, `EXPORT_DONE` at 150 s (34 ignores the dataSync knob, as RESULT says); transcript `timeout_knob.txt`: put 60000 → get 60000 → `dumpsys platform_compat | grep FGS_INTRODUCE_TIME_LIMITS` empty on 34 → delete "Successfully deleted" → null → sync `none` |
| `review-b2-guarded-a37-combo-auto-knob60` / `-datasync-knob60` | 37 | `FGS_ON_TIMEOUT` 61.07 / 61.06 s after HOME (SF→TO 69.9 s), lastTop→TO 60.02 s, HOME→lastTop 1.05 / 1.04 s; knob transcript shows the same full guard on 37 with `FGS_INTRODUCE_TIME_LIMITS; enableSinceTargetSdk=35` present |
| `review-b2-guarded-a37-exhaust-1st-timeout` / `-2nd-foreground` | 37 | timeout (61.07 s) then a fresh foreground start times out again at 61.05 s, lastTop→TO 60.01 / 60.02 s, gaps 1.05 / 1.03 s — D37/D38 reproduce inside the corrected bands |
| `review-b2-a31` queues | 31 | `amendments` set ran all five; `timeouts` set runs no cases on 31 (below the API-34 floor — by set definition); `set_sync_disabled_for_tests`/`get_sync_disabled_for_tests` do not exist on 31 (`Invalid command`, live) → A2 §7 confirmed |
| `review-b2-b1apk-a{31,34,37}-combo-bad-type-name` | 31, 34, 37 | with the correct `apk-b1` binary: `RuntimeException←IllegalArgumentException` (uncaught, crash by design) recorded on all three levels via `fatal_exception_at_launch=` — A2's claim reproduced |

## 3. Documentation and source read (2026-11-13)

* Live: `datasync-migration` (data-transfer options), `fgs/troubleshooting`, `fgs/service-types`,
  `fgs-types-required` (Android 14). Saved copies for the two previously unsaved pages
  (`docs/datasync-migration.txt`, `docs/service-types-live.txt`); all RESULT quotes re-checked against
  the `docs/` copies as before.
* AOSP (saved this round): `SettingsProvider` `DeviceConfigService.java` android14/15/16-release,
  `SettingsToPropertiesMapper.java` android16-release (android14 copy already in `docs/`).
* The "Android 14 page" citation in *Not exercised* for the quote "in addition to requesting the
  `FOREGROUND_SERVICE` permission" → that sentence is on the service-types page (verbatim: "… request
  the appropriate foreground service permission for that type (in addition to requesting the
  FOREGROUND_SERVICE permission)"), not on the Android 14 page (live-checked) → D47.

## 4. Defects found and fixed

1. **D45 — three D37 figures printed 0.01 s high, range one frame wide.** In RESULT: the review-B1
   rows stated 61.08 s / 61.09 s / 61.12 s for `review-b1-a37-combo-auto-knob60`,
   `review-b1-a37-combo-datasync-knob60` and `review-b1-a37-exhaust-1st-timeout`; the device log lines
   give 61.07 / 61.08 / 61.11 (HOME `10:41:58.565` → `10:42:59.632` etc.). The stated range
   "61.05–61.12 s" → "61.05–61.11 s". Five places (two tables, one summary sentence, the six-run
   paragraph). My fresh guarded runs (61.05–61.07 s) confirm the corrected band, not the old one.
2. **D46 — D38 "left TOP 1.04–1.06 s after the HOME line" understates its own range.**
   `review-a2-a37-exhaust-2nd-foreground` left TOP 1.095 s (rounds to 1.10) after its HOME line
   (`device_time` 12:26:37.031, `lastTopTime=-29s124ms`); the other six runs sit at 1.04–1.06.
   Corrected to 1.04–1.10 s; "60.02 s after it in all seven runs" re-verified unchanged (fresh B2 runs
   sit at 1.03–1.05 s and 60.01–60.02 s — same shape, outside that sentence's enumerated seven).
3. **D47 — quote attributed to the wrong page** in *Not exercised* (see §3); corrected to the
   service-types page and link.
4. **D48 — `run_case.sh` kept stale background snapshots when a case label ran twice** (harness defect,
   found by my own re-runs). The in-background `dumpsys_bg.txt` / `resumed_bg.txt` / `processes_bg.txt`
   block is guarded by `! -f dumpsys_bg.txt`; a re-run into an existing case directory (which happens
   whenever a set is re-run under the same `CASE_TAG` — my `amendments` and `timeouts` sets share case
   names) kept the first pass's three files while `logcat_full.txt` and `case.txt` came from the second:
   mixed evidence inside one directory (e.g. a `processes_bg` header 17 min older than the same case's
   HOME line). Fix: `run_case.sh` now deletes the three files at case start. The six affected B2
   directories (34: `-full-short-timeout`, `-combo-datasync-knob60`; 37: `-combo-auto-knob60`,
   `-combo-datasync-knob60`, `-exhaust-1st-timeout`, `-exhaust-2nd-foreground` — the cases the
   `amendments` and `timeouts` sets share) were regenerated under `CASE_TAG=review-b2-guarded`; the
   stale trio is left in place but superseded (each case's guard-run directory is authoritative for
   the D37/D38 numbers).

No app source change; no earlier evidence or review file changed. RESULT.md changes: the D45 number
corrections (5 places), D46 range, D47 attribution.

## 5. Agreement with the recommendation

Agree, unchanged: declare `mediaProcessing|dataSync` with `FOREGROUND_SERVICE`,
`FOREGROUND_SERVICE_MEDIA_PROCESSING` and `FOREGROUND_SERVICE_DATA_SYNC`, and pass `mediaProcessing`
when `SDK_INT >= 35`, else `dataSync`, with round A1's three amendments and A2's wording corrections.
B2's fresh runs on 31, 34 and 37 agree with every conclusion; the D45/D46 corrections are in the
timeout-clock note and change no advice (stop the service when the export ends).

## 6. Open items

* **`Pixel_6_API_29` still does not boot** (unchanged since B1/A2: `There must be exactly one installer; found []`);
  its repair (`-wipe-data`) needs the owner's decision and was not touched here. API 29 claims therefore
  continue to rest on the A1/B1 evidence and the android10 source, as RESULT and A2 state; nothing new
  was run on 29 this round.
* Carried over, unchanged: API 30/32/33 untested; full 24 h window; background-start against an exhausted
  budget; `ServiceCompat.startForeground()`; everything in RESULT's "What the emulator cannot show" list.

## 7. Environment state after this round

* Every emulator this round started was tracked and stopped: `Pixel_6_API_37` (pids 6452, 81757),
  `Pixel_6_API_34` (71160, 81980 — started twice, second boot on a bound port), `Pixel_6_API_31`
  (64581), plus the 34 cleanup boot 82385 — all stopped via `adb emu kill`/`kill`; the final
  `ps ax | grep -E "qemu-system|emulator -avd"` is empty. Never more than two were up at a time
  (the transient third, 81980, failed to bind a console port and was killed within minutes).
* Per-AVD final state, each read back live before the last stop: `com.example.fgslab` absent on 31,
  34 and 37 (uninstall `Success` on each); `data_sync_fgs_timeout_duration`,
  `media_processing_fgs_timeout_duration` and `short_service_fgs_timeout_duration` all `null` on 34
  (and the two knob keys `null` on 37 and 31); `get_sync_disabled_for_tests` = `none` on 34 and 37
  (command absent on 31, as expected).
* `Pixel_6_API_29` was not booted or modified (still the owner's repair); `Pixel_10_Pro_XL` untouched.
* Evidence added this round: `evidence/review-b2-*` (queues), `evidence/review-b2-guarded-*` (fixed-harness
  re-runs), `evidence/review-b2-b1apk-*` (bad-type-name with the right binary), `logs/b2-*` (queue and
  emulator logs, `b2-timecheck.sh`, `b2-audit.sh`), docs additions listed in §3. `run_case.sh` carries the
  D48 fix. RESULT.md carries D45–D47. Earlier evidence and reviews are byte-unchanged.

**Verdict: 4 defects found and fixed (D45, D46, D47, D48); not clean.**
