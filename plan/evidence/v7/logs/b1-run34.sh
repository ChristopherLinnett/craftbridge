#!/bin/bash
cd /tmp/craftbridge-v7
./run_case.sh emulator-5554 apk-b1/fgslab-combo-ts37.apk review-b1-a34-combo-short-notdeclared shortService 45
CASE_TAG=review-b1 ./queue.sh emulator-5554 34 review-a1 /tmp/craftbridge-v7/apk-b1
