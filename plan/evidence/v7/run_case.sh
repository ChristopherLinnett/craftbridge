#!/bin/bash
# One FGS scenario: install apk, start activity (foreground start), background it,
# wait for outcome marker, capture logcat + dumpsys.
# usage: run_case.sh <serial> <apk> <label> <type> <duration_s>
SER=$1; APK=$2; LABEL=$3; TYPE=$4; DUR=$5
ADB="$HOME/Library/Android/sdk/platform-tools/adb"
PKG=com.example.fgslab
OUT=/tmp/craftbridge-v7/evidence/$LABEL
mkdir -p "$OUT"
# Re-run into an existing case dir (same label twice) must not inherit files from the
# earlier run: the background snapshot is guarded by ! -f dumpsys_bg.txt, and a run that
# dies at launch writes no dumpsys_early/late, so every file this script writes is removed
# first (timeout_knob.txt is the queue's and is written before this script runs).
rm -f "$OUT"/{am_start,install,logcat_full,dumpsys_early,dumpsys_bg,resumed_bg,processes_bg,dumpsys_late}.txt
# Device clock: host and guest clocks can drift apart (the API 29 guest once ran 4 min
# behind), so timings measured against device log lines need the device's own time.
devclock() { $ADB -s $SER shell date +%H:%M:%S.%N 2>/dev/null | tr -d '\r' | cut -c1-12; }
# Appended once at the end of every outcome: the app's target SDK (from its own
# SERVICE_CREATED line) and, if this app crashed, the exception classes of that crash.
record_end_fields() { # <fatal field name>
  # This run's pid; when the app died before pidof saw it, the pid of this run's own (last)
  # launch line. On API 29 the clear leaves the previous case's crash in the buffer, so only
  # a crash of this pid counts.
  local pid=$PID fatal
  [ -z "$pid" ] && pid=$(grep 'ACTIVITY_STARTED_FGS_FROM_FOREGROUND' "$OUT/logcat_full.txt" | tail -1 | sed -E 's/.*CraftFGS\( *([0-9]+)\).*/\1/')
  echo "target_sdk=$(grep "CraftFGS( *$pid)" "$OUT/logcat_full.txt" | grep -m1 -oE 'targetSdk=[0-9]+' | cut -d= -f2)" >> "$OUT/case.txt"
  # The exception line right after "Process: <pkg>, PID: <pid>" and its "Caused by:" lines, class names only.
  fatal=$(awk -v mark="Process: $PKG, PID: $pid" '
      found && /AndroidRuntime/ {
        line = $0; sub(/.*AndroidRuntime\( *[0-9]+\): /, "", line)
        if (n == 0 || line ~ /^Caused by: /) { sub(/^Caused by: /, "", line); sub(/:.*/, "", line); print line }
        if (++n > 80) exit
      }
      !found && index($0, mark) && substr($0, index($0, mark) + length(mark)) !~ /^[0-9]/ { found = 1 }' "$OUT/logcat_full.txt" 2>/dev/null | awk '!seen[$0]++' | tr '\n' ';' | sed 's/;$//')
  [ -n "$fatal" ] && echo "$1=$fatal" >> "$OUT/case.txt"
}
{
echo "case=$LABEL"
echo "apk=$APK"
echo "apk_md5=$(md5 -q "$APK")"
echo "serial=$SER device_sdk=$($ADB -s $SER shell getprop ro.build.version.sdk | tr -d '\r')"
echo "type=$TYPE duration_s=$DUR"
echo "start=$(date -Iseconds)"
} > "$OUT/case.txt"
# Clearing only the default buffers has left the previous case's EXPORT_DONE
# behind on API 29, so clear every buffer and also match outcomes by this run's pid.
$ADB -s $SER logcat -b all -c 2>/dev/null
$ADB -s $SER install -r -g "$APK" > "$OUT/install.txt" 2>&1
INSTALL_RC=$?
echo "install_rc=$INSTALL_RC" >> "$OUT/case.txt"
if [ $INSTALL_RC -ne 0 ]; then
  echo "result=INSTALL_FAILED end=$(date -Iseconds)" >> "$OUT/case.txt"
  $ADB -s $SER logcat -d -v time > "$OUT/logcat_full.txt" 2>&1
  echo "$LABEL INSTALL_FAILED"; exit 1
fi
$ADB -s $SER shell am force-stop $PKG
$ADB -s $SER shell am start -W -n $PKG/.MainActivity \
  --es fgs_type "$TYPE" --el fgs_duration_ms $((DUR*1000)) > "$OUT/am_start.txt" 2>&1 &
AM=$!
# `am start -W` never returns on API 29 when the app crashes during launch, so bound the wait.
for _ in $(seq 1 30); do kill -0 $AM 2>/dev/null || break; sleep 1; done
if kill -0 $AM 2>/dev/null; then kill $AM; echo "am_start_wait=gave_up_after_30s" >> "$OUT/case.txt"; fi
PID=$($ADB -s $SER shell pidof $PKG 2>/dev/null | tr -d '\r')
echo "pid=$PID" >> "$OUT/case.txt"
if [ -z "$PID" ]; then
  # The app died during launch (e.g. a crash in onStartCommand): nothing to poll.
  echo "result=PROCESS_GONE_AT_LAUNCH end=$(date -Iseconds)" >> "$OUT/case.txt"
  $ADB -s $SER logcat -d -v time > "$OUT/logcat_full.txt" 2>&1
  # Record *why* it died: a crash can otherwise hide behind this generic label.
  record_end_fields fatal_exception_at_launch
  $ADB -s $SER shell am force-stop $PKG
  echo "$LABEL PROCESS_GONE_AT_LAUNCH"; exit 0
fi
sleep 8
$ADB -s $SER shell dumpsys activity services $PKG > "$OUT/dumpsys_early.txt" 2>&1
echo "home_sent_device=$(devclock)" >> "$OUT/case.txt"
$ADB -s $SER shell input keyevent KEYCODE_HOME
echo "home_sent=$(date -Iseconds)" >> "$OUT/case.txt"
BG_SNAPSHOT_AT=$(( $(date +%s) + 30 ))
END=$(( $(date +%s) + DUR + 150 ))
RESULT="WAIT_TIMEOUT"
while [ "$(date +%s)" -lt "$END" ]; do
  sleep 10
  if [ ! -f "$OUT/dumpsys_bg.txt" ] && [ "$(date +%s)" -ge "$BG_SNAPSHOT_AT" ]; then
    # Proves the app really is in the background while its service keeps running.
    $ADB -s $SER shell dumpsys activity services $PKG > "$OUT/dumpsys_bg.txt" 2>&1
    $ADB -s $SER shell dumpsys activity activities | grep -E 'mResumedActivity|topResumedActivity|ResumedActivity:' > "$OUT/resumed_bg.txt" 2>&1
    # Process state incl. its last-TOP time, relative to the device time noted first.
    { echo "device_time=$(devclock)"; $ADB -s $SER shell dumpsys activity processes $PKG; } > "$OUT/processes_bg.txt" 2>&1
  fi
  L=$($ADB -s $SER logcat -d --pid="$PID" -s CraftFGS 2>/dev/null)
  if echo "$L" | grep -q "EXPORT_DONE"; then RESULT="EXPORT_DONE"; break; fi
  if echo "$L" | grep -q "STARTFOREGROUND_FAIL"; then RESULT="STARTFOREGROUND_FAIL"; sleep 3; break; fi
  if echo "$L" | grep -q "FGS_ON_TIMEOUT"; then RESULT="FGS_ON_TIMEOUT"; sleep 20; break; fi
  # On API 29 a crashed process stays alive behind its crash dialog, so look for the crash itself.
  if $ADB -s $SER logcat -d --pid="$PID" -s AndroidRuntime 2>/dev/null | grep -q "FATAL EXCEPTION"; then
    RESULT="APP_CRASHED"; break
  fi
  P=$($ADB -s $SER shell pidof $PKG 2>/dev/null | tr -d '\r')
  if [ "$P" != "$PID" ]; then
    if echo "$L" | grep -q "STARTFOREGROUND_OK"; then RESULT="PROCESS_GONE_MIDRUN"; else RESULT="PROCESS_GONE_NO_SF_OK"; fi
    break
  fi
done
echo "result=$RESULT end=$(date -Iseconds)" >> "$OUT/case.txt"
$ADB -s $SER logcat -d -v time > "$OUT/logcat_full.txt" 2>&1
record_end_fields fatal_exception
$ADB -s $SER shell dumpsys activity services $PKG > "$OUT/dumpsys_late.txt" 2>&1
$ADB -s $SER shell am force-stop $PKG
echo "$LABEL $RESULT"
