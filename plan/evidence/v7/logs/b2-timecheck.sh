#!/bin/bash
# B2 timing re-computation from saved evidence (device clock throughout).
# HOME = "ActivityStartInterceptor: Starting home with component specified" log line.
# TIMEOUT = "FGS (...) timed out" / "Short FGS timed out" log line.
E=/tmp/craftbridge-v7/evidence
hms2sec() { awk -F: -v t="$1" 'BEGIN{split(t,a,":"); printf "%.3f", a[1]*3600+a[2]*60+a[3]}'; }
ts() { grep -m1 "$1" "$2" | awk '{print $2}'; }
lasttop_abs() {
  local dt lt sec
  dt=$(grep -m1 '^device_time=' "$1/processes_bg.txt" | cut -d= -f2)
  lt=$(grep -m1 -o 'lastTopTime=-[0-9]*s[0-9]*ms' "$1/processes_bg.txt" | sed 's/lastTopTime=-//')
  sec=$(echo "$lt" | sed -E 's/([0-9]+)s([0-9]+)ms/\1.\2/')
  awk -v d="$(hms2sec "$dt")" -v o="$sec" 'BEGIN{printf "%.3f", d-o}'
}
for c in "$@"; do
  d=$E/$c
  [ -d "$d" ] || { echo "$c MISSING"; continue; }
  HL=$(ts 'Starting home with component specified' "$d/logcat_full.txt")
  TL=$(grep -m1 -E 'FGS \([a-zA-Z]+\) timed out|Short FGS timed out' "$d/logcat_full.txt" | awk '{print $2}')
  SL=$(grep -m1 'STARTFOREGROUND_OK' "$d/logcat_full.txt" | awk '{print $2}')
  printf "%-52s" "$c"
  [ -n "$HL" ] && [ -n "$TL" ] && awk -v a="$(hms2sec "$HL")" -v b="$(hms2sec "$TL")" 'BEGIN{printf "HOME->TO=%7.2fs ", b-a}'
  [ -n "$SL" ] && [ -n "$TL" ] && awk -v a="$(hms2sec "$SL")" -v b="$(hms2sec "$TL")" 'BEGIN{printf "SF->TO=%7.2fs ", b-a}'
  if [ -f "$d/processes_bg.txt" ] && [ -n "$TL" ]; then
    LT=$(lasttop_abs "$d")
    awk -v lt="$LT" -v to="$(hms2sec "$TL")" 'BEGIN{printf "lastTop->TO=%6.2fs ", to-lt}'
    [ -n "$HL" ] && awk -v lt="$LT" -v h="$(hms2sec "$HL")" 'BEGIN{printf "HOME->lastTop=%5.2fs ", lt-h}'
  fi
  echo ""
done
