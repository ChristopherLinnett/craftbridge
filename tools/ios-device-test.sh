#!/usr/bin/env bash
# Runs a package's cargo test binaries on the acceptance iPhone and reports each binary's result.
#
#   tools/ios-device-test.sh <package>… [-- <test harness flags>]
#
# Each test binary is built for aarch64-apple-ios, wrapped in an app, signed, installed, launched with
# `xcrun devicectl`, its output captured, and uninstalled. The harness flags (a filter, `--nocapture`,
# `--ignored`) reach the app as launch arguments with no `--` before them: devicectl would pass that `--`
# to the app too, and the harness then reads every later word as a test name filter. The runner always
# passes `--test-threads=1`, so pass none.
#
# What the phone and the signing take comes from the environment, never from the repository:
#   CRAFTBRIDGE_DEVICE_ID              the CoreDevice identifier of the phone (`xcrun devicectl list devices`)
#   CRAFTBRIDGE_SIGNING_IDENTITY       the codesign identity (`security find-identity -v -p codesigning`)
#   CRAFTBRIDGE_PROVISIONING_PROFILE   the development profile file, which names the team and the phone
#   CRAFTBRIDGE_DEVICE_LOGS            where each launch's output is kept (default: a new folder under $TMPDIR)
# The tests are built as the gate builds, so a switch the gate refuses (tools/env.sh) is refused here too.
#
# A binary the system kills when it runs whole (an app has a memory limit a test binary's whole run can pass)
# is finished one test per launch: the tests with no result in the whole run's output each get an app launch
# of their own, and one the system kills alone is a failure. The tests tools/device-tests.tsv names are not
# run as they stand, each for its reason there; a test that reruns itself in a child process has the child
# run in a launch of its own instead. A launch that runs past its limit is ended like a killed one, so a test
# that hangs fails instead of holding the run; one whose app never starts its harness is tried again, and
# stops the run if it still does not start.
#
# Exit status 0 when every binary passed, 1 when a test failed, a binary could not run or no test ran at all.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"
# Before anything is built: a RUSTFLAGS, for one, would replace the device's CPU target set below.
craftbridge_refuse_switches || exit 1

target=aarch64-apple-ios
# Every app this runner installs has a bundle id under this prefix, and it never touches another app.
bundle_prefix=io.github.christopherlinnett.craftbridge.devtest.
excluded_tests=tools/device-tests.tsv
# The longest one launch may run, in seconds, before devicectl abandons it as a failure (its own
# `--timeout`): many times what any launch has needed, so only a test that hangs reaches it.
launch_limit=1800
phone_hint="If the phone is locked, unlock it; if it asks to trust this computer, trust it; then run again"

die() {
  echo "ios-device-test: $*" >&2
  exit 1
}

packages=()
while [ "$#" -gt 0 ] && [ "$1" != -- ]; do
  packages+=("$1")
  shift
done
if [ "$#" -gt 0 ]; then shift; fi
harness_flags=("$@")
[ "${#packages[@]}" -gt 0 ] || die "usage: tools/ios-device-test.sh <package>… [-- <test harness flags>]"

for variable in CRAFTBRIDGE_DEVICE_ID CRAFTBRIDGE_SIGNING_IDENTITY CRAFTBRIDGE_PROVISIONING_PROFILE; do
  [ -n "${!variable:-}" ] || die "$variable is not set: see the header of tools/ios-device-test.sh"
done
[ -r "$CRAFTBRIDGE_PROVISIONING_PROFILE" ] || die "cannot read the profile $CRAFTBRIDGE_PROVISIONING_PROFILE"
for tool in jq xcrun codesign security; do
  command -v "$tool" >/dev/null 2>&1 || die "no $tool on PATH"
done
device="$CRAFTBRIDGE_DEVICE_ID"
plist_buddy=/usr/libexec/PlistBuddy

# What a line of the table may hold, each problem on a line `<file>:<line>: <problem>`: seven fields, none
# empty since a shell read collapses an empty one, a kind the runner knows, a reason, and a child test only
# for a rerun, with its environment as NAME=value words or `-` for none. A misspelt kind would otherwise
# leave its test unrun with no word of why, and a rerun's child never launched.
table_problems() {
  awk -F'\t' '
    /^#/ || NF == 0 { next }
    {
      where = FILENAME ":" FNR ": "
      if (NF != 7) { print where NF " fields, not seven"; next }
      for (i = 1; i <= NF; i++) if ($i == "") { print where "field " i " is empty: write - where a field does not apply"; next }
      if ($4 != "host-path" && $4 != "host-tool" && $4 != "rerun") { print where "no kind " $4 ": host-path, host-tool or rerun"; next }
      if ($7 == "-") print where "no reason"
      if ($4 == "rerun" && ($3 == "*" || $5 == "-")) print where "a rerun names its test and the child test it reruns"
      if ($4 != "rerun" && ($5 != "-" || $6 != "-")) print where "only a rerun names a child test and its environment"
      if ($4 == "rerun" && $6 != "-") {
        n = split($6, words, " ")
        for (i = 1; i <= n; i++) if (words[i] !~ /^[A-Za-z_][A-Za-z0-9_]*=/) print where "the child environment " words[i] " is no NAME=value word"
      }
    }' "$excluded_tests"
}
problems="$(table_problems)"
if [ -n "$problems" ]; then
  printf '%s\n' "$problems" | sed 's/^/ios-device-test: /' >&2
  die "$excluded_tests is read only as its header says"
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-device-work.XXXXXX")"
logs="${CRAFTBRIDGE_DEVICE_LOGS:-$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-device-logs.XXXXXX")}"
mkdir -p "$logs"
installed=""
launch_status=0

# An app still on the phone when the run ends, however it ends, is uninstalled.
uninstall() {
  if [ -n "$installed" ]; then
    xcrun devicectl device uninstall app --device "$device" "$installed" >"$logs/uninstall.log" 2>&1 </dev/null ||
      echo "ios-device-test: could not uninstall $installed from the phone: see $logs/uninstall.log" >&2
    installed=""
  fi
}
craftbridge_remove_on_exit "$work" uninstall
trap 'exit 1' INT TERM

# The team the profile belongs to: the profile is a signed plist, which the system's own tools read.
profile_plist="$work/profile.plist"
security cms -D -i "$CRAFTBRIDGE_PROVISIONING_PROFILE" >"$profile_plist" || die "cannot read the profile $CRAFTBRIDGE_PROVISIONING_PROFILE"
team="$("$plist_buddy" -c 'Print :TeamIdentifier:0' "$profile_plist")" || die "the profile names no team"

# The rows of the table of excluded tests for a package's test binary, as `kind<TAB>test<TAB>child<TAB>environment`.
excluded_rows() {
  awk -F'\t' -v package="$1" -v binary="$2" '!/^#/ && $1 == package && $2 == binary { print $4 "\t" $3 "\t" $5 "\t" $6 }' "$excluded_tests"
}

# The log of the launch that runs one test of an app alone: `test_log <app name> <test> <ending>`.
test_log() {
  printf '%s/%s.%s.%s' "$logs" "$1" "$(printf '%s' "$2" | tr -c 'A-Za-z0-9_' '-')" "$3"
}

# One app launch of the wrapped test binary: `launch <log> <environment word>… -- <harness argument>…`, each
# environment word NAME=value. The output, the console of the app, goes to the log.
launch() {
  local log="$1" word
  local -a environment=()
  shift
  while [ "$1" != -- ]; do
    word="$1"
    environment+=("DEVICECTL_CHILD_$word")
    shift
  done
  shift
  launch_status=0
  env ${environment[@]+"${environment[@]}"} xcrun devicectl device process launch --console --terminate-existing \
    --timeout "$launch_limit" --device "$device" "$installed" "$@" >"$log" 2>&1 </dev/null || launch_status=$?
  # The console ends each line with a carriage return, which no line of the harness's output should keep.
  tr -d '\r' <"$log" >"$log.lf" && mv "$log.lf" "$log"
}

# Whether a launch's app got going: the list ended (`listed`), or the harness said how many tests it runs
# (`started`), which it does before it runs the first. `<check> <log>`.
# shellcheck disable=SC2329 # named to `launched` below, which runs it
listed() { [ "$launch_status" = 0 ]; }
# shellcheck disable=SC2329 # named to `launched` below, which runs it
started() { grep -Eq '^running [0-9]+ tests?$' "$1"; }

# A launch whose app never got going (the phone slept or locked, or its connection reset) ran nothing, so it
# is tried again, up to three times, and then stops the run, since no test after it would run either:
# `launched <check> <what failed> <log> <environment word>… -- <harness argument>…`.
launched() {
  local check="$1" what="$2" log="$3" attempt=1
  shift 3
  while :; do
    launch "$log" "$@"
    "$check" "$log" && return 0
    [ "$attempt" -lt 3 ] || break
    attempt=$((attempt + 1))
    sleep 5
  done
  cat "$log" >&2
  die "$what on the phone: see $log. $phone_hint"
}

# The tests a launch listed, one name a line.
listed_tests() {
  sed -n 's/^\(.*\): test$/\1/p' "$1"
}

# Wrap one test binary in an app, sign it, and install it: `install_app <app name> <executable>`.
install_app() {
  local name="$1" executable="$2" app="$work/$1.app" bundle minimum
  bundle="$bundle_prefix$name"
  mkdir -p "$app"
  cp "$executable" "$app/$name"
  cp "$CRAFTBRIDGE_PROVISIONING_PROFILE" "$app/embedded.mobileprovision"
  # The system version the binary was built to run on, which the app's plist must not claim to exceed.
  minimum="$(xcrun vtool -show-build "$executable" | awk '$1 == "minos" || $1 == "version" { print $2; exit }')"
  [ -n "$minimum" ] || die "cannot read the system version $executable was built for"
  rm -f "$app/Info.plist" "$work/entitlements.plist"
  "$plist_buddy" -c "Add :CFBundleIdentifier string $bundle" -c "Add :CFBundleExecutable string $name" \
    -c "Add :CFBundleName string $name" -c "Add :CFBundleVersion string 1" -c "Add :CFBundleShortVersionString string 1" \
    -c "Add :CFBundlePackageType string APPL" -c "Add :MinimumOSVersion string $minimum" \
    -c "Add :UIDeviceFamily array" -c "Add :UIDeviceFamily:0 integer 1" \
    -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string iPhoneOS" "$app/Info.plist" >/dev/null
  "$plist_buddy" -c "Add :application-identifier string $team.$bundle" -c "Add :com.apple.developer.team-identifier string $team" \
    -c "Add :get-task-allow bool true" "$work/entitlements.plist" >/dev/null
  codesign --force --sign "$CRAFTBRIDGE_SIGNING_IDENTITY" --entitlements "$work/entitlements.plist" "$app" >&2 || die "cannot sign $name"
  if ! xcrun devicectl device install app --device "$device" "$app" >"$logs/$name.install.log" 2>&1 </dev/null; then
    cat "$logs/$name.install.log" >&2
    die "cannot install $name on the phone. $phone_hint"
  fi
  installed="$bundle"
}

# The counts of a harness's last `test result:` line in a log, `passed failed`, or nothing when it has none
# (the app was killed before the harness finished).
result_of() {
  sed -n 's/^test result: [A-Za-z]*\. \([0-9]*\) passed; \([0-9]*\) failed;.*/\1 \2/p' "$1" | tail -n 1
}

# Run one test binary, wrapped as the installed app: `run_binary <package> <target> <executable>`. Prints one
# line of verdict and sets `passed` and `failed` for the caller.
run_binary() {
  local package="$1" binary="$2" executable="$3" name
  local universe filtered all_selected log result row kind test child environment
  local -a skips=() selected=() launches=() children=() children_env=() child_environment=()
  name="$package-${binary//_/-}"
  passed=0
  failed=0
  # A binary whose every test needs the host is not installed at all.
  if excluded_rows "$package" "$binary" | awk -F'\t' '$2 == "*" { found = 1 } END { exit !found }'; then
    echo "  not run: every test of $name (the table names it whole)"
    return 0
  fi
  install_app "$name" "$executable"

  universe="$logs/$name.list.log"
  launched listed "$name did not list its tests" "$universe" -- --list --format terse
  filtered="$universe"
  if [ "${#harness_flags[@]}" -gt 0 ]; then
    filtered="$logs/$name.filtered.log"
    launched listed "$name did not list its tests" "$filtered" -- --list --format terse "${harness_flags[@]}"
  fi

  # Each excluded test must still be a test of the binary, named by exactly one of its tests, so a rename
  # cannot leave one unexcluded or another excluded with it.
  while IFS=$'\t' read -r kind test child environment; do
    [ -n "$kind" ] || continue
    [ "$(listed_tests "$universe" | grep -Fxc -- "$test" || true)" = 1 ] || die "$excluded_tests names $test, which $name does not list: see $universe"
    [ "$(listed_tests "$universe" | grep -Fc -- "$test" || true)" = 1 ] || die "$excluded_tests names $test, a name that is part of another test of $name"
    if listed_tests "$filtered" | grep -Fxq -- "$test"; then
      skips+=(--skip "$test")
      if [ "$kind" = rerun ]; then
        children+=("$child")
        # `-`: the parent starts its child with nothing added to the environment.
        if [ "$environment" = - ]; then children_env+=(""); else children_env+=("$environment"); fi
      fi
      echo "  not run: $test ($kind)"
    fi
  done < <(excluded_rows "$package" "$binary")

  all_selected="$(listed_tests "$filtered")"
  while IFS= read -r test; do
    [ -n "$test" ] || continue
    row="$(excluded_rows "$package" "$binary" | awk -F'\t' -v test="$test" '$2 == test')"
    [ -n "$row" ] || selected+=("$test")
  done <<<"$all_selected"

  if [ "${#selected[@]}" -gt 0 ]; then
    log="$logs/$name.log"
    launched started "$name did not start its tests" "$log" -- --test-threads=1 ${harness_flags[@]+"${harness_flags[@]}"} ${skips[@]+"${skips[@]}"}
    result="$(result_of "$log")"
    if [ -n "$result" ]; then
      passed="${result% *}"
      failed="${result#* }"
    else
      # The system ended the app before the harness did. Whatever the output shows finished keeps its
      # result; every other test, the one that was running included, is launched alone.
      echo "  the app ended before its harness did: the rest of $name runs one test per launch"
      passed="$(grep -c '^test .* \.\.\. ok$' "$log" || true)"
      failed="$(grep -c '^test .* \.\.\. FAILED$' "$log" || true)"
      for test in "${selected[@]}"; do
        if ! grep -Fxq -- "test $test ... ok" "$log" && ! grep -Fxq -- "test $test ... FAILED" "$log" &&
          ! grep -Fxq -- "test $test ... ignored" "$log"; then
          launches+=("$test")
        fi
      done
      for test in ${launches[@]+"${launches[@]}"}; do
        log="$(test_log "$name" "$test" log)"
        launched started "$name did not start $test" "$log" -- --test-threads=1 ${harness_flags[@]+"${harness_flags[@]}"} --exact "$test"
        result="$(result_of "$log")"
        if [ -z "$result" ]; then
          echo "  FAILED $test: the app ended before its harness did, run alone (killed by the system, or past the launch limit): see $log"
          failed=$((failed + 1))
        else
          passed=$((passed + ${result% *}))
          failed=$((failed + ${result#* }))
          [ "${result#* }" = 0 ] || echo "  FAILED $test"
        fi
      done
    fi
    grep '^test .* \.\.\. FAILED$' "$logs/$name.log" | sed 's/^test \(.*\) \.\.\. FAILED$/  FAILED \1/' || true
  fi

  for row in "${!children[@]}"; do
    child="${children[$row]}"
    log="$(test_log "$name" "$child" child.log)"
    # The words split with no pathname expansion: a value is the environment's, never a file's name.
    read -r -a child_environment <<<"${children_env[$row]}"
    launched started "$name did not start the child test $child" "$log" ${child_environment[@]+"${child_environment[@]}"} -- --test-threads=1 --exact --ignored "$child"
    result="$(result_of "$log")"
    if [ "$result" = "1 0" ]; then
      passed=$((passed + 1))
    else
      echo "  FAILED $child (the child of a test that reruns itself): see $log"
      failed=$((failed + 1))
    fi
  done
  uninstall
}

craftbridge_ios_target_env "$target"
total_tests=0
status=0
echo "ios-device-test: the output of each launch is kept in $logs"
for package in "${packages[@]}"; do
  build="$(craftbridge_cargo test --no-run --locked --target "$target" --package "$package" --message-format=json-render-diagnostics)" ||
    die "the build of $package for $target failed"
  binaries="$(printf '%s\n' "$build" | jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) | "\(.target.name)\t\(.executable)"' | sort -u)"
  [ -n "$binaries" ] || die "$package builds no test binary for $target"
  while IFS=$'\t' read -r binary executable; do
    echo "$package $binary"
    run_binary "$package" "$binary" "$executable"
    total_tests=$((total_tests + passed + failed))
    if [ "$failed" -gt 0 ]; then
      echo "$package $binary: FAILED ($passed passed, $failed failed)"
      status=1
    elif [ "$passed" -gt 0 ]; then
      echo "$package $binary: ok ($passed passed)"
    else
      echo "$package $binary: no test ran"
    fi
  done <<<"$binaries"
done
[ "$total_tests" -gt 0 ] || die "no test ran on the phone"
craftbridge_passed=1
exit "$status"
