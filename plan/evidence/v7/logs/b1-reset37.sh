#!/bin/bash
cd /tmp/craftbridge-v7
S=emulator-5556; ADB="$HOME/Library/Android/sdk/platform-tools/adb -s $S"
D=evidence/review-b1-a37-datasync-exhaust-reset; mkdir -p $D
{ echo "\$ adb shell device_config put activity_manager data_sync_fgs_timeout_duration 60000"; $ADB shell device_config put activity_manager data_sync_fgs_timeout_duration 60000
  echo "\$ adb shell device_config get activity_manager data_sync_fgs_timeout_duration"; $ADB shell device_config get activity_manager data_sync_fgs_timeout_duration; } > $D/knob.txt
./run_case.sh $S apk-b1/fgslab-combo-ts37.apk review-b1-a37-exhaust-1st-timeout dataSync 300
./run_case.sh $S apk-b1/fgslab-combo-ts37.apk review-b1-a37-exhaust-2nd-foreground dataSync 45
{ echo "\$ adb shell device_config delete activity_manager data_sync_fgs_timeout_duration"; $ADB shell device_config delete activity_manager data_sync_fgs_timeout_duration
  echo "\$ adb shell device_config get activity_manager data_sync_fgs_timeout_duration"; $ADB shell device_config get activity_manager data_sync_fgs_timeout_duration; } >> $D/knob.txt
cp evidence/review-b1-a37-exhaust-1st-timeout/case.txt evidence/review-b1-a37-exhaust-2nd-foreground/case.txt $D/ 2>/dev/null
echo "RESET_TEST DONE"
