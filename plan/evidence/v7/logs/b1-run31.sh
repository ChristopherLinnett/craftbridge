#!/bin/bash
cd /tmp/craftbridge-v7
ADB="$HOME/Library/Android/sdk/platform-tools/adb"
for i in $(seq 1 60); do [ "$($ADB -s emulator-5556 shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ] && break; sleep 5; done
echo "31 booted sdk=$($ADB -s emulator-5556 shell getprop ro.build.version.sdk | tr -d '\r')"
SMOKE_PREFIX=review-b1 ./queue.sh emulator-5556 31 smoke /tmp/craftbridge-v7/apk-b1
echo "RUN31 DONE"
