#!/bin/bash
cd /tmp/craftbridge-v7
ADB="$HOME/Library/Android/sdk/platform-tools/adb"
for i in $(seq 1 60); do [ "$($ADB -s emulator-5554 shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ] && break; sleep 5; done
SDK=$($ADB -s emulator-5554 shell getprop ro.build.version.sdk | tr -d '\r'); echo "29 booted sdk=$SDK"
[ "$SDK" = "29" ] || { echo "ABORT: not 29"; exit 1; }
./run_case.sh emulator-5554 apk-b1/fgslab-combo-ts37.apk review-b1-a29-combo-none none 45
./run_case.sh emulator-5554 apk-b1/fgslab-combo-ts37.apk review-b1-a29-combo-short-notdeclared shortService 45
./run_case.sh emulator-5554 apk-b1/fgslab-combo-ts37.apk review-b1-a29-combo-mediaproc mediaProcessing 45
echo "RUN29 DONE"
