#!/usr/bin/env bash
# Runs a package's cargo test binaries, or the Swift package's Swift Testing suite, on the acceptance iPhone and
# reports each binary's, or each test's, result.
#
#   tools/ios-device-test.sh <package>… [-- <test harness flags>]
#   tools/ios-device-test.sh --swift [-- <Swift Testing flags>]
#
# Each test binary is built for aarch64-apple-ios, wrapped in an app, signed, installed, launched with
# `xcrun devicectl` (started stopped and then resumed, so the system's 20-second allowance for an app's launch,
# which a test binary never completes, does not end a long run: see `launch`), its output captured, and
# uninstalled. The harness flags (a filter, `--nocapture`, `--ignored`) reach the app as launch arguments with
# no `--` before them: devicectl would pass that `--` to the app too, and the harness then reads every later
# word as a test name filter. The runner always passes `--test-threads=1`, so pass none.
#
# What the phone and the signing take comes from the environment, never from the repository:
#   CRAFTBRIDGE_DEVICE_ID              the CoreDevice identifier of the phone (`xcrun devicectl list devices`)
#   CRAFTBRIDGE_SIGNING_IDENTITY       the codesign identity (`security find-identity -v -p codesigning`)
#   CRAFTBRIDGE_PROVISIONING_PROFILE   the development profile file, which names the team and the phone
#   CRAFTBRIDGE_DEVICE_LOGS            where each launch's output is kept (default: a new folder under $TMPDIR)
#   CRAFTBRIDGE_DEVICE_TEST_LIMIT      the seconds a Swift test may run before it counts as hung (default 300)
# The tests are built as the gate builds, so a switch the gate refuses (tools/env.sh) is refused here too.
#
# Two runs on one phone take turns: a run takes the machine's lock for its device — the lock `cargo xtask
# simulator` takes per simulator, held through `cargo xtask device-lock` — before it builds anything or
# touches the phone, and a run that waits for another says so, naming the device. However a run ends —
# Ctrl-C, a kill of its id or of its terminal among them — it takes its apps off the phone before its lock
# is let go, and nothing it started is left to keep the phone busy.
#
# A binary the system ends before its harness does when it runs whole (past the app's memory limit, say) is
# finished one test per launch: the tests with no result in the whole run's output each get an app launch
# of their own, and one the system kills alone is a failure. The tests tools/device-tests.tsv names are not
# run as they stand, each for its reason there; a test that reruns itself in a child process has the child
# run in a launch of its own instead. A launch that runs past its limit is ended like a killed one, so a test
# that hangs fails instead of holding the run; one whose app never starts its harness is tried again, and
# stops the run if it still does not start.
#
# The Swift mode (`--swift`) builds the Swift package's tests for the device, as the gate's Swift builds do, and
# wraps the bundle of each of the package's test targets in the host app tools/ios-test-host.swift (compiled
# here with `swiftc`, no Xcode project), with the Swift Testing frameworks they link and `conformance/effects`,
# which `Corpus.file` reads from the app's bundle there. The host runs the suite through Swift Testing's entry
# point for tools and prints each test's result in the shape of a Rust harness's, which the steps below read as
# they read a Rust binary's. Its flags are Swift Testing's own (`--filter`, `--skip`), and the runner always passes
# `--no-parallel`, as the Rust runs pass `--test-threads=1`: no test runs beside the one that sends the app to
# the background, whose trip would otherwise interrupt a render or an export another test has in flight. The
# host keeps the screen from locking, since a locked phone suspends an app and every test in it hangs, and it
# fails by name a test that runs past CRAFTBRIDGE_DEVICE_TEST_LIMIT. A test that needs the app in the
# background prints the line the runner put in its environment (CRAFTBRIDGE_RUNNER_BACKGROUND_REQUEST), with
# ` for <n> seconds` after it for a stay of its own, and the runner then brings a second app of its own, the
# host's executable under another bundle id, to the front for those seconds (a few, by default) and brings the
# host back.
#
# Exit status 0 when every binary passed, 1 when a test failed, a binary could not run or no test ran at all,
# and 2 when the run refused to start at all, before building anything or taking the device's lock, because a
# scheme, test plan or package configuration git does not track, or a workspace or project beside the
# package's manifest, stands where xcodebuild and swift build read the package's schemes and resolve, or the
# machine's own SwiftPM configuration or the shell names a mirror.
set -euo pipefail
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
cd "$CRAFTBRIDGE_ROOT"
# This file and the whole argument list it came in with, kept because the run starts again under the
# device's lock below: after the cd only an absolute path names this file from wherever the restart
# stands, and after the parse below only a copy holds the arguments whole.
script="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
run_args=("$@")
# Before anything is built: a RUSTFLAGS, for one, would replace the device's CPU target set below.
craftbridge_refuse_switches || exit 1

target=aarch64-apple-ios
# Every app this runner installs has a bundle id under this prefix, and it never touches another app.
bundle_prefix=io.github.christopherlinnett.craftbridge.devtest.
excluded_tests=tools/device-tests.tsv
# The longest one launch may run, in seconds, before devicectl abandons it as a failure (its own
# `--timeout`): many times what any launch has needed, so only a test that hangs reaches it.
launch_limit=1800
# The seconds one Swift test may run before the host abandons it as hung and fails it by name: the launch limit
# above ends a run that hangs but cannot say which test did.
test_limit="${CRAFTBRIDGE_DEVICE_TEST_LIMIT:-300}"
# What a test of the Swift suite prints to ask for the app to be sent to the background: the runner tells the
# app the line, so it is defined here alone. `cover` is the bundle id of the second app that does it, which
# is on the phone for the run.
background_request="craftbridge-runner: send the app to the background"
# How long a request that names no stay keeps the app in the background.
cover_seconds=6
# Why an app asks to add to Photos, which the system shows with its prompt: the test that exports the running
# example's movie saves it there, and the owner watches it.
photos_reason="A device test saves the movie it exports to Photos, so the owner can watch it and judge its sound."
cover=""
phone_hint="If the phone is locked, unlock it; if it asks to trust this computer, trust it; then run again"

die() {
  echo "ios-device-test: $*" >&2
  exit 1
}

usage="usage: tools/ios-device-test.sh <package>… [-- <test harness flags>] | --swift [-- <Swift Testing flags>]"
swift_mode=no
packages=()
while [ "$#" -gt 0 ] && [ "$1" != -- ]; do
  if [ "$1" = --swift ]; then swift_mode=yes; else packages+=("$1"); fi
  shift
done
if [ "$#" -gt 0 ]; then shift; fi
harness_flags=("$@")
if [ "$swift_mode" = yes ]; then
  [ "${#packages[@]}" -eq 0 ] || die "$usage"
else
  [ "${#packages[@]}" -gt 0 ] || die "$usage"
fi

# Before anything is built and before the device's lock is taken: the Swift run asks `xcodebuild` for the
# package through `-scheme CraftBridge-Package`, so a scheme or test plan an Xcode session left where it
# reads the package's schemes, or a mirror or registry left in the package's own configuration folder, or a
# mirror the machine's own configuration or the shell names, which the run's resolve reads and fetches a
# dependency's sources through, would be what the run builds and runs on the phone, rather than what the
# manifest gives (tools/env.sh, which reads the package's places and the mirrors' itself).
# The runner builds no sample, so it names no project bundle, and a run of a package's cargo test binaries
# builds nothing a scheme can redirect. The run under the lock starts this file again and meets the check
# again, which then also covers a file Xcode wrote while the run waited for the lock. The refusal is status 2,
# not a test failure's 1: the run never started.
if [ "$swift_mode" = yes ]; then
  # shellcheck disable=SC2119 # the helper's arguments are the sample bundles a caller builds, and this builds none
  craftbridge_refuse_leftover_xcode_and_swiftpm_settings || exit 2
fi

for variable in CRAFTBRIDGE_DEVICE_ID CRAFTBRIDGE_SIGNING_IDENTITY CRAFTBRIDGE_PROVISIONING_PROFILE; do
  [ -n "${!variable:-}" ] || die "$variable is not set: see the header of tools/ios-device-test.sh"
done
[ -r "$CRAFTBRIDGE_PROVISIONING_PROFILE" ] || die "cannot read the profile $CRAFTBRIDGE_PROVISIONING_PROFILE"
# A limit the host could not read would leave a hung test to the launch limit, naming nothing.
[[ "$test_limit" =~ ^0*[1-9][0-9]*$ ]] || die "CRAFTBRIDGE_DEVICE_TEST_LIMIT is $test_limit, which is no whole number of seconds above 0"
for tool in jq xcrun codesign security; do
  command -v "$tool" >/dev/null 2>&1 || die "no $tool on PATH"
done
device="$CRAFTBRIDGE_DEVICE_ID"
plist_buddy=/usr/libexec/PlistBuddy

# The device this run names is one phone, and every run installs its app on it under the same bundle ids:
# two runs started at once replace each other's app mid-run, and the one that ends first uninstalls what
# the other is still testing on, so what fails is the runs and not the code. Therefore the run takes the
# machine's lock for this device here, from this one place it names it — the lock `cargo xtask simulator`
# takes per simulator, through `cargo xtask device-lock`, and nowhere a second time — and runs the rest of
# itself again under it: nothing is built and no phone is touched before the lock is held, and a run that
# waits for another says so naming the device. The lock is taken per name, so the phone is named one way
# only, as CoreDevice names it: its serial, its UDID or its name would each take another lock.
[[ "$device" =~ ^[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}$ ]] ||
  die "CRAFTBRIDGE_DEVICE_ID is $device, which is no CoreDevice identifier: name the phone as \`xcrun devicectl list devices\` does, since another name for it would take another lock"
# The run under the lock is the one `cargo xtask device-lock` started for this device: the variable it sets
# names the device and its own process id, which only its own child has for a parent, so neither a shell
# that exported the variable nor a run started inside a run passes for the run that holds the lock. This
# run stays the process its caller started, and its lock command a child of it in the background, which
# leaves that command deaf to a terminal's Ctrl-C and hang-up: a run that is interrupted, or killed by its
# id, ends here, and the lock command, seeing its caller gone, asks the run under the lock to end — which
# takes its apps off the phone — and lets the lock go once it has. The lock command is built for this run
# alone, in a folder this run removes however it ends but a kill outright.
if [ "${CRAFTBRIDGE_DEVICE_LOCK:-}" != "$device:$PPID" ]; then
  lock_run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-device-lock.XXXXXX")"
  trap 'rm -rf "$lock_run"' EXIT
  trap 'exit 1' INT TERM HUP
  craftbridge_xtask_build "$lock_run" || die "the build of the device-lock tool failed"
  (
    trap '' HUP
    exec "$lock_run/debug/xtask" device-lock "$device" -- bash "$script" "${run_args[@]+"${run_args[@]}"}"
  ) &
  lock_status=0
  wait "$!" || lock_status=$?
  exit "$lock_status"
fi

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
# Every app this run has put on the phone, by bundle id, and the one the launches below start.
on_phone=()
installed=""
# The installed app's executable as the phone names it, from the end of its path: `<bundle folder>/<executable>`.
installed_executable=""
launch_status=0

# Every app still on the phone when the run ends, however it ends, is uninstalled.
uninstall() {
  local bundle
  for bundle in ${on_phone[@]+"${on_phone[@]}"}; do
    xcrun devicectl device uninstall app --device "$device" "$bundle" >"$logs/uninstall.log" 2>&1 </dev/null ||
      echo "ios-device-test: could not uninstall $bundle from the phone: see $logs/uninstall.log" >&2
  done
  on_phone=()
  installed=""
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

# The process ids the phone lists for the installed app's executable, one a line, from the phone's own list of
# its processes; fails when the phone does not answer.
app_processes() {
  xcrun devicectl device info processes --device "$device" --json-output "$work/processes.json" >/dev/null 2>&1 </dev/null &&
    jq -r --arg executable "/$installed_executable" \
      '.result.runningProcesses[] | select(.executable | endswith($executable)) | .processIdentifier' "$work/processes.json"
}

# Resumes the app a launch started, for as long as the launch's devicectl, `<devicectl pid>`, waits on it:
# every process the phone lists for the app is resumed (one whose resume fails is tried again at the next
# read), and the list is read every second until one is resumed and every five seconds after. The list cannot
# tell the launch's own process from another of the app's: the phone may start another process of an app just
# after it ends, which stays listed for seconds, and one a failed launch left stopped stays listed until this
# launch's `--terminate-existing` ends it. Either may be listed alone before the launch's own process shows,
# and the first may show only after a list read before the launch; so resuming only the one process found, or
# stopping at the first resume or the first output (devicectl writes lines of its own into the log before the
# app starts), would leave the app stopped until the launch limit. Resuming the others too harms nothing, since
# each has ended, runs on, or is ended by `--terminate-existing`. Each resume's output goes to `<log>.resume`.
# `resume_app <devicectl pid> <log>` fails when no process of the app could be resumed in 60 reads of the
# list, or the launch ended with none resumed.
resume_app() {
  local launcher="$1" log="$2" reads=0 resumed="" pid pause
  while kill -0 "$launcher" 2>/dev/null; do
    if [ -z "$resumed" ] && [ "$reads" -ge 60 ]; then
      return 1
    fi
    reads=$((reads + 1))
    for pid in $(app_processes || true); do
      case "$resumed " in *" $pid "*) continue ;; esac
      if xcrun devicectl device process resume --device "$device" --pid "$pid" >>"$log.resume" 2>&1 </dev/null; then
        resumed+=" $pid"
      fi
    done
    pause=1
    [ -z "$resumed" ] || pause=5
    while [ "$pause" -gt 0 ] && kill -0 "$launcher" 2>/dev/null; do
      sleep 1
      pause=$((pause - 1))
    done
  done
  [ -n "$resumed" ]
}

# Serves what a running app asks of the runner through its output, while the launch's devicectl, `<devicectl
# pid>`, waits on it: an app that needs to be in the background prints `$background_request`, and the runner
# brings the cover app to the front, which sends the app to the background, and after the seconds the request
# names (`$background_request for <n> seconds`; `$cover_seconds` when it names none) the app itself, which
# returns. Each request is served once, in the order they are printed, and the log is read every
# second until the launch ends and once more after, so a request printed last is not lost. The log still has
# the console's carriage returns, so a line is matched by its start.
# `serve_requests <devicectl pid> <log>`.
serve_requests() {
  local launcher="$1" log="$2" served=0 asked running=yes stay
  while [ "$running" = yes ]; do
    kill -0 "$launcher" 2>/dev/null || running=no
    asked="$(grep -c "^$background_request" "$log" 2>/dev/null || true)"
    while [ "$served" -lt "${asked:-0}" ]; do
      served=$((served + 1))
      stay="$(grep "^$background_request" "$log" | sed -n "${served}s/^.* for \([0-9][0-9]*\) seconds[[:space:]]*\$/\1/p")"
      echo "ios-device-test: sending the app to the background for ${stay:-$cover_seconds} seconds (request $served)" >&2
      xcrun devicectl device process launch --device "$device" "$cover" >"$log.cover" 2>&1 </dev/null ||
        echo "ios-device-test: could not bring the cover app to the front: see $log.cover" >&2
      sleep "${stay:-$cover_seconds}"
      xcrun devicectl device process launch --device "$device" "$installed" >>"$log.cover" 2>&1 </dev/null ||
        echo "ios-device-test: could not bring the app back to the front: see $log.cover" >&2
    done
    [ "$running" = no ] || sleep 1
  done
}

# One app launch of the wrapped test binary: `launch <log> <environment word>… -- <harness argument>…`, each
# environment word NAME=value. The output, the console of the app, goes to the log.
#
# The app starts stopped, as for a debugger, and is resumed once it runs. FrontBoard ends a foreground app that
# has not finished launching 20 seconds after it started (its process-launch watchdog, 0x8BADF00D in the
# phone's crash report), and a test binary never finishes launching, since it never starts UIKit, so after a
# plain launch FrontBoard ends every run longer than that mid-test, whatever its tests do; a launch that waits
# for a debugger is not held to that allowance. The app still runs in the foreground, where it may use the GPU and
# the hardware encoders, as an app's export does.
launch() {
  local log="$1" word console server
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
    --start-stopped --timeout "$launch_limit" --device "$device" "$installed" "$@" >"$log" 2>&1 </dev/null &
  console=$!
  rm -f "$log.resume"
  serve_requests "$console" "$log" &
  server=$!
  if resume_app "$console" "$log"; then
    wait "$console" || launch_status=$?
  else
    # An app never resumed would wait, stopped, until the launch limit, so its launch ends here and counts as
    # one whose app never got going; the next launch's `--terminate-existing`, or the uninstall, ends the app.
    kill -KILL "$console" 2>/dev/null || true
    # Without the shell's own line that it killed the job, which says nothing the log does not.
    wait "$console" 2>/dev/null || true
    launch_status=1
    {
      [ ! -f "$log.resume" ] || cat "$log.resume"
      echo "ios-device-test: the app, started stopped, was never resumed"
    } >>"$log"
  fi
  rm -f "$log.resume"
  # The console has ended, so the server's last pass over the log is its last: it is waited for, never killed
  # with a request half served.
  wait "$server" || true
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

# The system version a binary was built to run on, which an app's plist must not claim to exceed, and which
# the host app is built for so that it runs wherever the test bundle it loads does: `built_for <binary>`.
built_for() {
  xcrun vtool -show-build "$1" | awk '$1 == "minos" || $1 == "version" { print $2; exit }'
}

# Wrap one test binary in an app, sign it, and install it: `install_app <app name> <executable> [<folder>]`.
# The folder's contents, if one is given, are put in the app first (frameworks, plug-ins, resources), and
# every framework, library and bundle in it is signed before the app, inside out, since the app's signature
# covers theirs.
install_app() {
  local name="$1" executable="$2" payload="${3:-}" app="$work/$1.app" bundle minimum nested attempt=1
  bundle="$bundle_prefix$name"
  mkdir -p "$app"
  cp "$executable" "$app/$name"
  if [ -n "$payload" ]; then
    cp -R "$payload/." "$app/"
    while IFS= read -r nested; do
      codesign --force --sign "$CRAFTBRIDGE_SIGNING_IDENTITY" "$nested" >&2 || die "cannot sign ${nested#"$app"/}"
    done < <(find "$app" -depth \( -name '*.framework' -o -name '*.dylib' -o -name '*.xctest' \))
  fi
  cp "$CRAFTBRIDGE_PROVISIONING_PROFILE" "$app/embedded.mobileprovision"
  minimum="$(built_for "$executable")"
  [ -n "$minimum" ] || die "cannot read the system version $executable was built for"
  rm -f "$app/Info.plist" "$work/entitlements.plist"
  "$plist_buddy" -c "Add :CFBundleIdentifier string $bundle" -c "Add :CFBundleExecutable string $name" \
    -c "Add :CFBundleName string $name" -c "Add :CFBundleVersion string 1" -c "Add :CFBundleShortVersionString string 1" \
    -c "Add :CFBundlePackageType string APPL" -c "Add :MinimumOSVersion string $minimum" \
    -c "Add :UIDeviceFamily array" -c "Add :UIDeviceFamily:0 integer 1" \
    -c "Add :CFBundleSupportedPlatforms array" -c "Add :CFBundleSupportedPlatforms:0 string iPhoneOS" \
    -c "Add :NSPhotoLibraryAddUsageDescription string $photos_reason" "$app/Info.plist" >/dev/null
  "$plist_buddy" -c "Add :application-identifier string $team.$bundle" -c "Add :com.apple.developer.team-identifier string $team" \
    -c "Add :get-task-allow bool true" "$work/entitlements.plist" >/dev/null
  codesign --force --sign "$CRAFTBRIDGE_SIGNING_IDENTITY" --entitlements "$work/entitlements.plist" "$app" >&2 || die "cannot sign $name"
  # A phone another run is using, or whose connection dropped, may refuse an install that the next try takes.
  while ! xcrun devicectl device install app --device "$device" "$app" >"$logs/$name.install.log" 2>&1 </dev/null; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt 3 ]; then
      cat "$logs/$name.install.log" >&2
      die "cannot install $name on the phone. $phone_hint"
    fi
    sleep 10
  done
  on_phone+=("$bundle")
  installed="$bundle"
  installed_executable="${app##*/}/$name"
}

# The counts of a harness's last `test result:` line in a log, `passed failed`, or nothing when it has none
# (the app was killed before the harness finished).
result_of() {
  sed -n 's/^test result: [A-Za-z]*\. \([0-9]*\) passed; \([0-9]*\) failed;.*/\1 \2/p' "$1" | tail -n 1
}

# What a launch's harness says it ran, into `passed` and `failed`: the counts of its `test result:` line, or,
# when the app was killed before the harness finished, the tests it said it finished; fails in that case.
# `read_results <log>`.
read_results() {
  local result
  result="$(result_of "$1")"
  if [ -n "$result" ]; then
    passed="${result% *}"
    failed="${result#* }"
    return 0
  fi
  passed="$(grep -c '^test .* \.\.\. ok$' "$1" || true)"
  failed="$(grep -c '^test .* \.\.\. FAILED$' "$1" || true)"
  return 1
}

# The tests a launch's harness says failed, a line each: `failed_tests <log>`.
failed_tests() {
  grep '^test .* \.\.\. FAILED$' "$1" | sed 's/^test \(.*\) \.\.\. FAILED$/  FAILED \1/' || true
}

# Run one test binary, wrapped as the installed app: `run_binary <package> <target> <executable>`. Prints one
# line of verdict and sets `passed` and `failed` for the caller.
run_binary() {
  local package="$1" binary="$2" executable="$3" name
  local universe filtered all_selected log result row kind test other child environment
  local -a skips=() selected=() launches=() alone=() children=() children_env=() child_environment=()
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
    if ! read_results "$log"; then
      # The system ended the app before the harness did. Whatever the output shows finished keeps its
      # result; every other test, the one that was running included, is launched alone.
      echo "  the app ended before its harness did: the rest of $name runs one test per launch"
      for test in "${selected[@]}"; do
        if ! grep -Fxq -- "test $test ... ok" "$log" && ! grep -Fxq -- "test $test ... FAILED" "$log" &&
          ! grep -Fxq -- "test $test ... ignored" "$log"; then
          launches+=("$test")
        fi
      done
      for test in ${launches[@]+"${launches[@]}"}; do
        log="$(test_log "$name" "$test" log)"
        # With `--exact` the caller's filters and every `--skip` match whole names, so a filter that is the whole
        # name of another test would run that test beside this one: a selected one would be counted twice, and one
        # the table leaves out would run where it cannot. Each of those is skipped by its name, so the launch runs
        # this test alone.
        alone=(${skips[@]+"${skips[@]}"})
        for other in "${selected[@]}"; do
          [ "$other" = "$test" ] || alone+=(--skip "$other")
        done
        launched started "$name did not start $test" "$log" -- --test-threads=1 ${harness_flags[@]+"${harness_flags[@]}"} \
          ${alone[@]+"${alone[@]}"} --exact "$test"
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
    failed_tests "$logs/$name.log"
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

# Copies into `<folder>` every library the test bundles link through `@rpath` that the iPhone platform ships for
# tests (Swift Testing, XCTest, which the bundle links whatever its sources import, and what they link), read
# from each binary's own load commands, transitively: `test_libraries <folder> <binary>…`. The phone holds none
# of them, and the app's loader finds them in its Frameworks folder. The rest are the system's own Swift
# libraries, which the loader finds on the phone through the binaries' `/usr/lib/swift` search path and the
# SDK has a stub of; a library that is neither stops the run with its name, since the bundle would not load.
test_libraries() {
  local developer sdk destination="$1" i=0 name source seen=""
  local -a pending
  shift
  pending=("$@")
  developer="$(xcrun --sdk iphoneos --show-sdk-platform-path)/Developer"
  sdk="$(xcrun --sdk iphoneos --show-sdk-path)"
  mkdir -p "$destination"
  while [ "$i" -lt "${#pending[@]}" ]; do
    while IFS= read -r name; do
      case " $seen " in *" $name "*) continue ;; esac
      seen+=" $name"
      case "$name" in
        */*) source="$developer/Library/Frameworks/${name%%/*}"
          [ -e "$source" ] || source="$developer/Library/PrivateFrameworks/${name%%/*}" ;;
        *) source="$developer/usr/lib/$name" ;;
      esac
      if [ ! -e "$source" ]; then
        [ -e "$sdk/usr/lib/swift/${name%.dylib}.tbd" ] ||
          die "${pending[$i]##*/} links @rpath/$name, which neither $developer nor the system holds"
        continue
      fi
      cp -R "$source" "$destination/"
      pending+=("$destination/$name")
    done < <(otool -L "${pending[$i]}" | sed -n 's/^[[:space:]]*@rpath\/\([^ ]*\) (.*/\1/p')
    i=$((i + 1))
  done
}

# Run the Swift package's Swift Testing suite on the phone, as the header says. Prints the lines the host
# printed that name a failure's cause, and sets `passed` and `failed` for the caller.
run_swift_suite() {
  local derived=ios/sdk/.build/device-tests products tests target xctest binary minimum payload host log
  passed=0
  failed=0
  # The package's binary target is the XCFramework nobody commits, and SwiftPM builds nothing without it.
  tools/build-xcframework.sh --keep-bindings >&2 || die "the XCFramework did not build"
  (cd ios/sdk && xcodebuild -scheme CraftBridge-Package -destination generic/platform=iOS -configuration Debug \
    -derivedDataPath .build/device-tests -quiet build-for-testing ARCHS=arm64 CODE_SIGNING_ALLOWED=NO) >&2 ||
    die "the build of the Swift package's tests for the device failed"
  products="$derived/Build/Products/Debug-iphoneos"
  # The bundles of the package's own test targets, as SwiftPM describes the package: the build folder keeps the
  # bundle of a test target the package has since lost or renamed, which would otherwise run among its tests.
  tests="$(cd ios/sdk && xcrun swift package describe --type json | jq -r '.targets[] | select(.type == "test") | .name')" ||
    die "SwiftPM did not describe the package's test targets"
  [ -n "$tests" ] || die "the package has no test target"
  payload="$work/payload"
  mkdir -p "$payload/PlugIns" "$payload/conformance"
  while IFS= read -r target; do
    [ -d "$products/$target.xctest" ] || die "no test bundle $target.xctest in $products"
    cp -R "$products/$target.xctest" "$payload/PlugIns/"
  done <<<"$tests"
  cp -R conformance/effects "$payload/conformance/effects"
  for xctest in "$payload"/PlugIns/*.xctest; do
    binary="$xctest/$(basename "$xctest" .xctest)"
    test_libraries "$payload/Frameworks" "$binary"
    minimum="$(built_for "$binary")"
  done
  host="$work/host"
  xcrun --sdk iphoneos swiftc -swift-version 5 -target "arm64-apple-ios$minimum" tools/ios-test-host.swift -o "$host" \
    -Xlinker -rpath -Xlinker @executable_path/Frameworks >&2 ||
    die "cannot build tools/ios-test-host.swift"
  install_app swift-tests.cover "$host"
  cover="$installed"
  install_app swift-tests "$host" "$payload"

  log="$logs/swift-tests.log"
  launched started "swift-tests did not start its tests" "$log" "CRAFTBRIDGE_RUNNER_BACKGROUND_REQUEST=$background_request" \
    "CRAFTBRIDGE_RUNNER_TEST_LIMIT=$test_limit" -- \
    --no-parallel ${harness_flags[@]+"${harness_flags[@]}"}
  if ! read_results "$log"; then
    echo "  the app ended before its host did (killed by the system, or past the launch limit): see $log"
    failed=$((failed + 1))
  fi
  grep '^cases ' "$log" | sed 's/^cases \(.*\): \([0-9]*\)$/  \1: \2 cases/' || true
  grep '^the Swift run takes ' "$log" | sed 's/^/  /' || true
  grep '^failure ' "$log" | sed 's/^failure /  cause of /' || true
  failed_tests "$log"
  uninstall
}

total_tests=0
status=0
echo "ios-device-test: the output of each launch is kept in $logs"
# The XCFramework's own build sets the device CPU target, which the gate refuses to find already set.
if [ "$swift_mode" = yes ]; then
  echo "swift-tests"
  run_swift_suite
  echo "swift-tests: $passed passed, $failed failed"
  [ "$failed" -eq 0 ] && [ "$passed" -gt 0 ] || exit 1
  craftbridge_passed=1
  exit 0
fi
craftbridge_ios_target_env "$target"
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
