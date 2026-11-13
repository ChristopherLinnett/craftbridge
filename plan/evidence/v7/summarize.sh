#!/bin/bash
# Summarize evidence for RESULT.md: one line per case + key CraftFGS lines.
# Only lines from the case's own app process count: logcat -c has left the
# previous case's lines in the buffer on API 29.
E=/tmp/craftbridge-v7/evidence
for d in $(ls $E | sort); do
  [ -f "$E/$d/case.txt" ] || continue
  echo "===== $d ====="
  grep -E 'apk=|apk_md5=|device_sdk=|type=|install_rc|result=' "$E/$d/case.txt"
  PID=$(grep -m1 'ACTIVITY_STARTED_FGS' "$E/$d/logcat_full.txt" 2>/dev/null | sed -E 's/.*CraftFGS\( *([0-9]+)\).*/\1/')
  grep "CraftFGS( *$PID)" "$E/$d/logcat_full.txt" 2>/dev/null | grep -E 'STARTFOREGROUND|EXPORT_DONE|EXPORT_ABORTED|EXPORT_FAILED|FGS_ON_TIMEOUT|SERVICE_CREATED' | sed 's/^.*CraftFGS/[FGS]/'
  grep -m2 -E 'SecurityException|IllegalArgumentException|MissingForegroundServiceTypeException|ANR in|FATAL EXCEPTION|has died|Timeout' "$E/$d/logcat_full.txt" 2>/dev/null | head -4
  grep -m1 'foService\|isForeground=' "$E/$d/dumpsys_early.txt" 2>/dev/null | cut -c1-160
  echo
done
