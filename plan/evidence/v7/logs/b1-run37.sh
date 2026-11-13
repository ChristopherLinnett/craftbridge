#!/bin/bash
cd /tmp/craftbridge-v7
./run_case.sh emulator-5556 apk-b1/fgslab-combo-ts37.apk review-b1-a37-combo-short-notdeclared shortService 45
CASE_TAG=review-b1 ./queue.sh emulator-5556 37 review-a1 /tmp/craftbridge-v7/apk-b1
