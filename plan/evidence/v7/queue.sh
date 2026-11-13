#!/bin/bash
# Drives one device through a set of cases (each one run_case.sh call).
# usage: queue.sh <serial> <api> <set> <apk-dir>
#   set     matrix    the per-level type matrix of RESULT.md (labels a<api>-...)
#           review-a1 re-runs the recommendation rests on, plus adb-shortened
#                     type timeouts (labels review-a1-a<api>-...)
#           smoke     short checks of a rebuilt APK set (labels smoke-a<api>-...)
#           amendments the cases behind the three amendments of review A1, per level:
#                     type-less start, undeclared type, explicit types, 660 s run (every
#                     level), and on 34+ the
#                     adb-shortened timeouts incl. exhaustion then a foreground restart
#                     (labels amendments-a<api>-...)
#           timeouts  only the time-limit part of amendments (labels timeouts-a<api>-...)
#   apk-dir holds fgslab-<flavor>-ts37.apk; <apk-dir>-ts34 and <apk-dir>-ts33 hold the
#           target-SDK builds. Required, so a run can never fall back silently to a
#           superseded build.
#   CASE_TAG=<tag> labels every case <tag>-a<api>-... instead, so a later round re-runs a
#           set into new evidence directories and never overwrites an earlier round's.
# The device's SDK must equal <api>: a stale emulator on the same port once produced
# mislabelled evidence.
SER=$1; API=$2; SET=$3; D=$4
if [ -z "$D" ] || [ ! -f "$D/fgslab-combo-ts37.apk" ]; then
  echo "ABORT: apk-dir missing or without fgslab-combo-ts37.apk: '$D'"; exit 1
fi
ADB="$HOME/Library/Android/sdk/platform-tools/adb"
E=/tmp/craftbridge-v7/evidence
SDK=$($ADB -s $SER shell getprop ro.build.version.sdk | tr -d '\r')
if [ "$SDK" != "$API" ]; then echo "ABORT: $SER reports sdk=$SDK, expected $API"; exit 1; fi

F=$D/fgslab-full-ts37.apk; C=$D/fgslab-combo-ts37.apk
N=$D/fgslab-none-ts37.apk; P=$D/fgslab-noperm-ts37.apk
F34=$D-ts34/fgslab-full-ts34.apk; N34=$D-ts34/fgslab-none-ts34.apk; N33=$D-ts33/fgslab-none-ts33.apk

case $SET in
  matrix) PREFIX=a ;;
  review-a1) PREFIX=review-a1-a ;;
  smoke) PREFIX=smoke-a ;;
  amendments) PREFIX=amendments-a ;;
  timeouts) PREFIX=timeouts-a ;;
  *) echo "unknown set: $SET"; exit 1 ;;
esac
[ -n "$CASE_TAG" ] && PREFIX=${CASE_TAG}-a
Q() { /tmp/craftbridge-v7/run_case.sh $SER "$1" "$PREFIX$API-$2" "$3" "$4"; }

# A type timeout shortened to 60 s with adb (documented on the FGS timeout page). The
# device_config and compat state go to <file>; every key is read back after deletion.
# Play-services images can sync server flags into device_config mid-run and drop a local
# override (seen on API 37: the key vanished and the 60 s budget never applied), so server
# sync is suspended until the next reboot while a knob is set, and the key is read back
# before it is deleted to show it was still in force.
knob_on() { # <file> <device_config key>
  {
    echo "\$ adb shell device_config set_sync_disabled_for_tests until_reboot"
    $ADB -s $SER shell device_config set_sync_disabled_for_tests until_reboot 2>&1
    echo "\$ adb shell device_config put activity_manager $2 60000"
    $ADB -s $SER shell device_config put activity_manager "$2" 60000 2>&1
    echo "\$ adb shell device_config get activity_manager $2"
    $ADB -s $SER shell device_config get activity_manager "$2" 2>&1
    echo "\$ adb shell dumpsys platform_compat | grep FGS_INTRODUCE_TIME_LIMITS"
    $ADB -s $SER shell dumpsys platform_compat 2>&1 | grep -o 'ChangeId(.*FGS_INTRODUCE_TIME_LIMITS[^)]*)' || echo "(no such change id on this platform)"
  } > "$1"
}
knob_off() { # <file> <device_config key>
  {
    echo "\$ adb shell device_config get activity_manager $2   # still in force?"
    $ADB -s $SER shell device_config get activity_manager "$2" 2>&1
    echo "\$ adb shell device_config delete activity_manager $2"
    $ADB -s $SER shell device_config delete activity_manager "$2" 2>&1
    # A device that refuses to delete still has to end with the key unset.
    echo "\$ adb shell device_config get activity_manager $2"
    $ADB -s $SER shell device_config get activity_manager "$2" 2>&1
    echo "\$ adb shell device_config set_sync_disabled_for_tests none"
    $ADB -s $SER shell device_config set_sync_disabled_for_tests none 2>&1
    echo "\$ adb shell device_config get_sync_disabled_for_tests"
    $ADB -s $SER shell device_config get_sync_disabled_for_tests 2>&1
  } >> "$1"
}
QT() { # <apk> <case> <type> <duration_s> <device_config key>
  local dir=$E/$PREFIX$API-$2
  mkdir -p "$dir"
  knob_on "$dir/timeout_knob.txt" "$5"
  Q "$1" "$2" "$3" "$4"
  knob_off "$dir/timeout_knob.txt" "$5"
}
# Exhausts a 60 s dataSync budget, then starts the type again from the foreground.
QX() { # <apk>
  local dir=$E/$PREFIX$API-datasync-exhaust-knob
  mkdir -p "$dir"
  knob_on "$dir/timeout_knob.txt" data_sync_fgs_timeout_duration
  Q "$1" exhaust-1st-timeout dataSync 300
  Q "$1" exhaust-2nd-foreground dataSync 150
  knob_off "$dir/timeout_knob.txt" data_sync_fgs_timeout_duration
}

if [ "$SET" = matrix ]; then
  Q $F full-none none 45
  Q $F full-datasync dataSync 45
  Q $F full-mediaproc mediaProcessing 45
  Q $F full-specialuse specialUse 45
  if [ "$API" -ge 34 ]; then Q $F full-short-timeout shortService 600; else Q $F full-short shortService 45; fi
  Q $N noneflavor-none none 45
  [ "$API" -ge 34 ] && Q $C combo-short-notdeclared shortService 45
  Q $P noperm-datasync dataSync 45
  Q $P noperm-mediaproc mediaProcessing 45
  if [ "$API" = 34 ]; then
    Q $P noperm-specialuse specialUse 45
    Q $N34 none-ts34-none none 45
    Q $N33 none-ts33-none none 45
  fi
  [ "$API" -ge 34 ] && Q $F34 full-ts34-mediaproc mediaProcessing 45
  Q $C combo-auto-long auto 660
  if [ "$API" -ge 34 ]; then Q $F full-specialuse-long specialUse 660; else Q $N noneflavor-long none 660; fi
elif [ "$SET" = review-a1 ]; then
  Q $C combo-mediaproc mediaProcessing 45
  Q $C combo-datasync dataSync 45
  Q $C combo-none none 45
  if [ "$API" -ge 35 ]; then
    QT $C combo-auto-knob60 auto 300 media_processing_fgs_timeout_duration
    QT $C combo-datasync-knob60 dataSync 300 data_sync_fgs_timeout_duration
  else
    [ "$API" -ge 34 ] && Q $F full-short-timeout shortService 600
    QT $C combo-datasync-knob60 dataSync 150 data_sync_fgs_timeout_duration
  fi
  Q $C combo-auto-long auto 660
elif [ "$SET" = amendments ] || [ "$SET" = timeouts ]; then
  if [ "$SET" = amendments ]; then
    Q $C combo-none none 45
    Q $C combo-short-notdeclared shortService 45
    Q $C combo-mediaproc mediaProcessing 45
    Q $C combo-datasync dataSync 45
    Q $C combo-bad-type-name bogus 45
  fi
  if [ "$API" -ge 35 ]; then
    QT $C combo-auto-knob60 auto 300 media_processing_fgs_timeout_duration
    QT $C combo-datasync-knob60 dataSync 300 data_sync_fgs_timeout_duration
    QX $C
  elif [ "$API" -ge 34 ]; then
    Q $F full-short-timeout shortService 600
    QT $C combo-datasync-knob60 dataSync 150 data_sync_fgs_timeout_duration
  fi
  # The one-APK claim covers every level, so the 660 s export runs on every level too.
  [ "$SET" = amendments ] && Q $C combo-auto-long auto 660
else
  Q $C combo-auto auto 45
  Q $C combo-mediaproc mediaProcessing 45
  Q $C combo-bad-type-name bogus 45
  [ "$API" -ge 35 ] && QT $C combo-auto-knob60 auto 300 media_processing_fgs_timeout_duration
fi
$ADB -s $SER uninstall com.example.fgslab
echo "QUEUE_${SET}_$API DONE"
