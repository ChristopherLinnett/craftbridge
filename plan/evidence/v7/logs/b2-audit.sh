#!/bin/bash
# B2 comprehensive evidence audit
E=/tmp/craftbridge-v7/evidence
echo "### 1. install Success + install_rc everywhere:"
tot=0; fail=0
for d in $E/*/; do
  [ -f "$d/install.txt" ] || continue
  tot=$((tot+1))
  grep -q Success "$d/install.txt" || { echo "NO-Success: $d"; fail=$((fail+1)); }
  if [ -f "$d/case.txt" ]; then grep -q 'install_rc=0' "$d/case.txt" || { echo "RC!=0: $d"; fail=$((fail+1)); }; fi
done
echo "install.txt dirs=$tot failures=$fail"

echo "### 2. fileKiB per long-run claim:"
for c in a29-combo-auto-long a29-combo-final a31-combo-auto-long a31-combo-final a34-combo-auto-long review-a1-a34-combo-auto-long review-b1-a34-combo-auto-long review-a2-a34-combo-auto-long a37-combo-auto-long review-a1-a37-combo-auto-long review-b1-a37-combo-auto-long review-a2-a37-combo-auto-long a29-noneflavor-long review-a1-a29-noneflavor-long a31-noneflavor-long a34-full-specialuse-long a37-full-specialuse-long; do
  l=$(grep -h -m1 'EXPORT_DONE' $E/$c/logcat_full.txt 2>/dev/null | sed 's/.*CraftFGS//')
  echo "$c: $l"
done

echo "### 3. FATAL EXCEPTION zero-claims:"
for c in a34-full-none a34-noneflavor-none a34-noperm-datasync a34-noperm-mediaproc a34-noperm-specialuse a37-noperm-datasync a37-noperm-mediaproc review-b1-a34-combo-none review-b1-a37-combo-none review-a2-a34-combo-none review-a2-a34-combo-mediaproc review-a2-a37-combo-none review-a2-a37-combo-mediaproc review-a2-a37-combo-datasync-knob60; do
  n=$(grep -c 'FATAL EXCEPTION' $E/$c/logcat_full.txt 2>/dev/null)
  echo "$c fatal=$n"
done

echo "### 4. dumpsys_early types lines:"
for c in a34-full-none a37-full-none a34-full-datasync review-b1-a34-combo-datasync review-a2-a34-combo-datasync review-b1-a37-combo-auto-long review-a2-a37-combo-auto-long review-b1-a37-combo-none review-a2-a34-combo-none; do
  printf "%-40s " "$c"; grep -m1 -oE 'types=[0-9a-fx]+' $E/$c/dumpsys_early.txt 2>/dev/null
  printf "%-40s bg: " ""; grep -m1 -oE 'types=[0-9a-fx]+' $E/$c/dumpsys_bg.txt 2>/dev/null
done
echo "### 4b. isForeground in dumpsys_bg of key long runs:"
for c in review-b1-a34-combo-auto-long review-a2-a34-combo-auto-long review-b1-a37-combo-auto-long review-a2-a37-combo-auto-long a29-noneflavor-none review-a1-a29-noneflavor-long; do
  printf "%-42s " "$c"; grep -m1 -oE 'isForeground=[a-z]+' $E/$c/dumpsys_bg.txt 2>/dev/null
done

echo "### 5. dumpsys_late '(nothing)' check (long runs):"
for c in a34-combo-auto-long a37-combo-auto-long review-b1-a34-combo-auto-long review-b1-a37-combo-auto-long review-a2-a34-combo-auto-long review-a2-a37-combo-auto-long; do
  printf "%-42s " "$c"; head -c 120 $E/$c/dumpsys_late.txt 2>/dev/null | tr -s ' \n' ' '; echo
done

echo "### 6. special/spot claims:"
echo "review-a1-a29-full-short-240: $(grep -h -m1 'EXPORT_DONE' $E/review-a1-a29-full-short-240/logcat_full.txt 2>/dev/null | sed 's/.*CraftFGS//')"; grep -c 'TICK' $E/review-a1-a29-full-short-240/logcat_full.txt 2>/dev/null
echo "a29-full-short TICKs:"; grep -h 'TICK' $E/a29-full-short/logcat_full.txt 2>/dev/null | sed 's/.*CraftFGS//' | head -3
echo "a34-full-ts34-mediaproc targetSDK:"; grep -m1 -o 'targetSDK=[0-9]*' $E/a34-full-ts34-mediaproc/logcat_full.txt
echo "a34-none-ts34-none exc:"; grep -m1 -oE 'MissingForegroundServiceTypeException[^*]{0,80}' $E/a34-none-ts34-none/logcat_full.txt | head -c 150; echo
echo "a34-none-ts34-none targetSDK:"; grep -m1 -o 'targetSDK=[0-9]*' $E/a34-none-ts34-none/logcat_full.txt
echo "a34-none-ts33-none:"; grep -h -m1 'SERVICE_CREATED' $E/a34-none-ts33-none/logcat_full.txt | sed 's/.*CraftFGS//'; grep -h -m1 'STARTFOREGROUND_OK' $E/a34-none-ts33-none/logcat_full.txt | sed 's/.*CraftFGS//'
echo "a29-combo-final SERVICE_CREATED:"; grep -m1 'SERVICE_CREATED' $E/a29-combo-final/logcat_full.txt | sed 's/.*CraftFGS//'
echo "a31-combo-final SERVICE_CREATED:"; grep -m1 'SERVICE_CREATED' $E/a31-combo-final/logcat_full.txt | sed 's/.*CraftFGS//'
echo "exhaust-reset dir:"; ls -d $E/review-b1-a37-datasync-exhaust-* 2>/dev/null
echo "### 7. shortService 240s claim TICK elapsedS=180:"
grep -h 'TICK elapsedS=180' $E/review-a1-a29-full-short-240/logcat_full.txt | sed 's/.*CraftFGS//' | head -2
echo "### 8. a34-full-none SHORT_SERVICE-ignored log + types:"
grep -m1 'SHORT_SERVICE will be ignored' $E/a34-full-none/logcat_full.txt | head -c 160; echo
echo "### 9. knob results (timeout fires under sync suspension, B1/a2-guarded):"
for c in review-a2-guarded-a34-combo-datasync-knob60 review-b1-a34-combo-datasync-knob60 review-a2-guarded-a37-combo-datasync-knob60 review-a2-guarded-a37-combo-auto-knob60; do
  printf "%-50s " "$c"; grep -m1 -h 'result=' $E/$c/case.txt
done
echo "### 10. review-a2 bad-type-name fatal_exception fields:"
for c in review-a2-a31-combo-bad-type-name review-a2-a34-combo-bad-type-name review-a2-a37-combo-bad-type-name; do
  printf "%-42s " "$c"; grep -h fatal_exception $E/$c/case.txt
done
echo "### 11. a31 no such command claim check (from A2/B1 logs):"
grep -l "set_sync_disabled_for_tests" $E/review-a2-a31*/timeout_knob.txt 2>/dev/null | head -3
echo "### 12. resumed_bg shows launcher for bg-proof cases:"
for c in review-b1-a34-combo-auto-long review-a2-a37-combo-auto-long; do printf "%-42s " "$c"; head -c 110 $E/$c/resumed_bg.txt | tr -s ' \n' ' '; echo; done