#!/usr/bin/env bash
# Stages a release of the SDK: the one version every artifact shares, the version's checks, and the tag.
# It stages only, and never publishes: it pushes nothing, tags nothing remotely, makes no GitHub release
# and uploads no package; publishing is the owner's alone to decide (plan §26, G2).
#
#   tools/release.sh --dry-run        rehearse the release in a throwaway clone of the tree, the tree's
#                                     changes committed there: nothing of the tree or its repository is
#                                     written (the build tools' shared caches are used as any build uses
#                                     them), and the clone goes when the run ends
#   tools/release.sh --stage          the owner's run: stage the release in the repository itself, at its
#                                     fetched main's tip, which tags that commit X.Y.Z there, unpushed,
#                                     and keeps what it staged — the proof repository, the XCFramework
#                                     zip, the distribution package — in the repository's ignored
#                                     release/X.Y.Z, for its owner to publish from.
#                                     A run names one of the two, so no run tags a repository it was not
#                                     told to. The run holds the machine's release lock on the repository
#                                     for its whole life: a release stages one version of one tree once,
#                                     so a second staging run of it is refused, naming the lock, not
#                                     queued — its reads would have moved by the time it could start
#   … --repo <path>                   the CraftBridge repository to stage or rehearse in place of this
#                                     checkout, so a rehearsal can stand after release tags this checkout
#                                     does not hold
#
# The one version (D-27) is the one written in `rust/Cargo.toml`: Cargo answers it for every crate of the
# workspace, and every one of them must answer the same one, since the artifacts share one native library
# and are never mixed. The same string goes to Gradle on the command line as the `craftbridge.version`
# property every publish task refuses to run without (android/sdk/build-logic), where it outranks every
# other place Gradle reads a property from, and comes back read from each published AAR's own module
# metadata. It is the tag as well. A Swift package carries its version in no file: SwiftPM takes it from
# the tag of the repository it resolves the package from, and it resolves a package only at a repository's
# root, which ios/sdk is not, so an app takes the Swift package from the distribution repository a release
# writes from ios/sdk (D-24), at that repository's own tag. What this repository's tag versions is the
# Swift package's source: it names the commit whose ios/sdk the package is released from, and is read back
# from git, with the package's manifest in that commit's tree.
#
# What a release may be is settled before anything is built. CHANGELOG.md must still hold the release's
# rows under [Unreleased] — that section is what becomes the version's — and no section for the version,
# which would say it was released once. Each row stands under a heading of its kind, written by hand
# (AGENTS.md §11), and the headings give the release its class: a row under `### Breaking changes`, a change
# that breaks what a release shipped, takes the next major version, one under `### Added` the next minor,
# and the rest a minor or a patch; a major version with no breaking change is refused too, since major is
# what breaking changes take. Only the headings named below stand there, and no row stands under none, so
# no misspelt heading hides a break. A first release is 0.1.0 and holds no `### Breaking changes`: no
# release stands before it to break. Past it a version is the next one of its class after the newest
# release, (major+1).0.0, major.(minor+1).0 or major.minor.(patch+1), and tags are exactly X.Y.Z.
#
# The steps run in the one list at the end of this script, each standing alone. The tag is the last step
# that writes, and the run's one write to the repository's history: every step before it reads the
# repository, or writes only the run's own folder and the build folders git ignores, and a run that does
# not pass removes the tag it made, so a run that fails, or is stopped, at any step leaves no tag. The
# release's artifacts stand in that list now: the XCFramework zip with the checksum SwiftPM will verify,
# and the Swift distribution package written from ios/sdk, which is what an app resolves (D-24). The
# release notes, and the copies of the AARs they ship with, join that list before the tag. Every step
# takes the version from `version`, which one step reads once and no step can change, so a release runs
# this one script start to finish and no step moves for their coming.
set -euo pipefail
# Git exports its repository variables (GIT_DIR and the rest) to a hook and to whatever `git rebase --exec`
# or `git bisect run` starts. Every git command here must act on the repository the run was given.
# shellcheck disable=SC2046 # one variable name per word, split on purpose
unset $(git rev-parse --local-env-vars 2>/dev/null)
# shellcheck source=tools/env.sh
source "$(dirname "$0")/env.sh"
# A `--repo` path is the caller's, so it is read from the folder the run started in.
caller="$PWD"
# The script itself, named so the name outlives the moving of the next line: a staging run starts itself
# again under the lock, by this name.
script="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
cd "$CRAFTBRIDGE_ROOT"

usage() {
  echo "usage: tools/release.sh --dry-run|--stage [--repo <path>] — rehearse the release rust/Cargo.toml versions in a throwaway clone, or stage it in the repository, which tags it there (the owner's run); the script's header tells what each form does" >&2
}

# Every line the script says, in its one shape. A rehearsal says so in the shape.
say() { printf 'release%s: %s\n' "$run_kind" "$1"; }
fail() { say "$1"; exit 1; }

run_kind="" mode="" repo=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run | --stage)
      # A run is one of the two: a staging run that also meant to rehearse would tag what it only tried.
      [ -z "$mode" ] || {
        usage
        exit 64
      }
      mode="$1"
      ;;
    --repo)
      shift
      # A run stages or rehearses one repository, named by a path: a second name contradicts the first,
      # and an empty one would silently name this checkout, which is the repository --repo was given to
      # stand away from.
      [ $# -gt 0 ] && [ -n "$1" ] && [ -z "$repo" ] || {
        usage
        exit 64
      }
      repo="$1"
      ;;
    *)
      usage
      exit 64
      ;;
  esac
  shift
done
# A run that names neither tags nothing: a repository is tagged only when its owner asks for it.
[ -n "$mode" ] || {
  usage
  exit 64
}
# Every tool a release cannot do without, named before anything is read or written, since a release
# without one would otherwise fail only at the step that first calls it, far from its cause. The message
# names the tool and no fix: tools/doctor.sh reports none of jq, git and zip, and the swift it reaches
# only through the Xcode it checks.
for tool in jq git swift zip; do
  command -v "$tool" >/dev/null 2>&1 || fail "no $tool on PATH"
done

src="${repo:-$CRAFTBRIDGE_ROOT}"
top="$(git -C "$caller" -C "$src" rev-parse --show-toplevel 2>/dev/null)" || fail "$src is no git repository"
[ -f "$top/rust/Cargo.toml" ] || fail "$src holds no rust/Cargo.toml: it is no CraftBridge repository"
src="$top"
# A release stages one version of one tree once: two staging runs of one repository read the same newest
# release and both mean to tag the version after it, and a run that fails takes back what it reads as its
# own tag — so the second would refuse the first's version mid-flight, or undo its tag, in the one place
# either writes: the repository. So a staging run holds the machine's lock on the repository for its
# whole life, through `cargo xtask release-lock` — the machine's one lock, the same `cargo xtask
# device-lock` and `cargo xtask simulator` hold theirs through, not a second one — and takes it before
# reading or writing anything of the release. A second run is refused at its start, in words naming the
# lock, and not queued: waiting would only start it on the version, the changelog and the tip the first
# has just moved. The lock stands for the repository and not this checkout — every linked worktree of one
# shares the very tags a staging run writes — so it is named by the repository's one shared git folder,
# its path hashed because a lock file takes a name and no path. A rehearsal writes nothing to a repository
# and so takes no lock.
if [ "$mode" = --stage ]; then
  common="$(git -C "$src" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" ||
    fail "$src names no git folder of its own to lock while it is staged"
  git_common="$(cd "$common" 2>/dev/null && pwd -P)" || fail "$src's git folder $common is gone"
  lock_name="$(printf '%s' "$git_common" | git hash-object --stdin)"
  # The run under the lock is the one the lock command started this script for: the variable it set names
  # the lock and its own process, of which only the script it started has that process for a parent — so
  # neither an exported variable nor a run started inside a run passes for the holder. The command runs in
  # the background, so this run stays the caller it watches: interrupted, this run ends, and it ends the
  # run under the lock after that run's own cleanup, and lets the lock go only then.
  if [ "${CRAFTBRIDGE_RELEASE_LOCK:-}" != "$lock_name:$PPID" ]; then
    lock_bin="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-release-lock.XXXXXX")"
    # The lock tool's folder goes as this run ends, through env.sh's one exit trap: a plain removal would
    # end this run on the 0 bash 3.2 hands an unset variable's stop, and read as a staged release.
    craftbridge_remove_on_exit "$lock_bin"
    trap 'exit 1' HUP INT TERM
    craftbridge_xtask_build "$lock_bin" || fail "the build of the release-lock tool failed"
    say "staging $src under release lock $lock_name"
    (
      # A hangup belongs to the terminal ending this run, not to the run under the lock; that one is
      # ended by the command above, which asks it its own way.
      trap '' HUP
      exec "$lock_bin/debug/xtask" release-lock "$lock_name" -- bash "$script" --stage --repo "$src"
    ) &
    run_status=0
    wait "$!" || run_status=$?
    # This run's own part, starting the run under the lock, is done: it ends as that run did.
    craftbridge_passed=1
    exit "$run_status"
  fi
fi
# The commit the release tags: what stands committed in a staging run, and the rehearsed tree's own
# commit in a rehearsal.
commit="$(git -C "$src" rev-parse -q --verify 'HEAD^{commit}')" || fail "$src has no commit to release"

# Everything a run makes stands in one folder of its own — a rehearsal's clone, the build folders and the
# proof Maven repository, the XCFramework zip with the written distribution package — which ends as its
# mode promises: a rehearsal's folder goes however the run ends, since nothing a rehearsal writes stands
# to be kept, while a staging run's folder is the release it staged, and a passed staging run's moves into
# the repository's own ignored release folder, `release/<version>`, named at the end, for its owner to
# publish from — publishing is the owner's alone (plan §26, G2), so the run stages everything and
# publishes nothing. The folder the run works in stands in the temporary folder, which the system empties
# of what nobody has opened for days, so a staged release never stays there: the owner's publish may come
# days later. The tag a run makes goes with
# it when the run does not pass: a release stopped or refused after its tag is no release, and its tag
# would read as one, so the next run would refuse the version as released. A run has passed only when it
# said so, ended on 0 and left its folder as its mode promises: the pass mark and the status the exit
# trap records (tools/env.sh) are read as the trap reads them, and since the trap removes the run's
# folder after this function, the folder goes or moves here first. A pass whose folder would not go or
# move has to say its own failure, which a failing cleanup says — the trap reads it as a run that left
# something behind, and ends even a 0-status run non-zero; a run already failing has its own status to
# speak, so this function only fails for a pass that ended wrongly, and a failed run keeps no tag either.
tag_made=""
staged=""
# shellcheck disable=SC2329 # tools/env.sh's exit trap calls it by the name handed to craftbridge_remove_on_exit
removing_a_failed_runs_tag() {
  if [ -z "$tag_made" ]; then
    return 0
  fi
  if [ "${craftbridge_passed:-}" = 1 ] && [ "$craftbridge_status" = 0 ] && ending_the_run_folder; then
    # The pass ended its folder as its mode promised: there is nothing left to take back.
    return 0
  fi
  removing_the_tag_if_still_ours
  # A staging run that did not end as promised keeps nothing staged, what a move left half-made included:
  # the version's release folder stood nowhere when the run began (settling_where_the_release_stands),
  # and the release lock kept every other staging run off it since. What a rehearsal leaves behind, the
  # trap's own removal of the run's folder fails over, which fails the run the same way.
  if [ "$mode" = --stage ]; then
    rm -rf "$run"
    if [ -n "${move_began:-}" ]; then
      rm -rf "$staged"
    fi
  fi
  if [ "${craftbridge_passed:-}" = 1 ] && [ "$craftbridge_status" = 0 ]; then
    # The run's last words said where its release would stand; these take them back.
    say "the release could not be moved to stand in ${staged:-$run}: nothing is kept staged and the tag ${tag_made} is taken back, so the run fails"
    return 1
  fi
  return 0
}
# shellcheck disable=SC2329 # reached through removing_a_failed_runs_tag, which its own trap-call note keeps quiet
removing_the_tag_if_still_ours() {
  # Only the tag naming this run's commit is this run's own to take back: a tag of the same name on
  # another commit is somebody else's release and stays, and a run stopped between naming the tag
  # here and git making it has made no tag to take back.
  if [ "$(git -C "$repo" rev-parse -q --verify "refs/tags/$tag_made^{commit}")" = "$commit" ]; then
    git -C "$repo" tag -d "$tag_made" >/dev/null
  fi
}
# shellcheck disable=SC2329 # reached through removing_a_failed_runs_tag, which its own trap-call note keeps quiet
ending_the_run_folder() {
  # A rehearsal's folder goes with it: a folder that will not go is a run that failed, and fails its tag
  # too. A staging run's folder is its staged release, and it moves to its release folder on the pass: a
  # move that cannot be made is a release that cannot stand staged where its owner will look, so that run
  # fails too and keeps no tag, exactly as a rehearsal's stubborn folder does. A folder standing there
  # already would take the run inside it rather than be replaced, so the move is made only into nothing.
  if [ "$mode" = --dry-run ]; then
    rm -rf "$run"
  else
    rm -rf "${run:?}/$run_temporary" || return 1
    if [ -e "$staged" ]; then
      return 1
    fi
    mkdir -p "$(dirname "$staged")" || return 1
    # From here a failed move may have left part of the release in its folder, which is this run's own.
    move_began=1
    mv "$run" "$staged"
  fi
}
run="$(mktemp -d "${TMPDIR:-/tmp}/craftbridge-release.XXXXXX")"
craftbridge_remove_on_exit "$run" removing_a_failed_runs_tag

# A rehearsal works on a clone, so nothing of the tree or its repository is written. The changes are read
# with git's plumbing, which never writes the index back, as `git diff` and `git status` refresh it, and
# with renames read as the removal and the addition they are, so no old path stays in the clone. The
# clone holds what was committed and the tree may hold more, which is what a rehearsal rehearses, so the
# changes are copied in and committed there: a tag names a commit, and the commit tagged is the tree that
# was built. The clone keeps no remote, so nothing it does can reach the repository it came from.
rehearsing_in_a_clone() {
  local file
  repo="$run/repo"
  git clone -q --no-checkout -- "$src" "$repo" || fail "the rehearsal could not clone $src"
  git -C "$repo" remote remove origin
  git -C "$repo" checkout -q --detach "$commit"
  while IFS= read -r -d '' file; do
    if [ -e "$src/$file" ]; then
      mkdir -p "$repo/$(dirname "$file")"
      cp "$src/$file" "$repo/$file"
    else
      rm -f "$repo/$file"
    fi
  done < <(
    git -C "$src" diff-index --name-only --no-renames -z "$commit"
    git -C "$src" ls-files --others --exclude-standard -z
  )
  git -C "$repo" add --all
  if ! git -C "$repo" diff --cached --quiet; then
    git -C "$repo" -c user.name="release rehearsal" -c user.email="rehearsal@invalid" \
      -c commit.gpgSign=false -c core.hooksPath=/dev/null commit -q -m "The tree the release rehearses"
    commit="$(git -C "$repo" rev-parse --verify 'HEAD^{commit}')"
  fi
}
if [ "$mode" = --dry-run ]; then
  run_kind=" (dry run)"
  # The rehearsal's cargo builds into its own folder, so nothing of the tree's build folder is written.
  export CARGO_TARGET_DIR="$run/cargo-target"
  rehearsing_in_a_clone
  say "rehearsing the tree of $src in a clone, as the commit $commit"
else
  [ -z "$(git -C "$src" status --porcelain)" ] || fail "$src has changes that are not committed: a release stages the commit it tags"
  # A release stamps main, and main moves only through a merged pull request (AGENTS.md §12): a tree
  # below its tip releases less than main ships, and a tree above it stamps work no review has merged.
  # The script never fetches — a fetch is the owner's read of the repository, not the release's write —
  # so the tip it holds to is the one this repository last fetched; a stale fetch reads as a stale tip.
  origin_main="$(git -C "$src" rev-parse -q --verify 'refs/remotes/origin/main^{commit}')" ||
    fail "$src holds no fetched origin/main: a release stages main's tip, so fetch main into it first"
  [ "$commit" = "$origin_main" ] ||
    fail "$src does not stand at the tip of origin/main, which stands at $origin_main: a release stages main's tip, not $commit"
  repo="$src"
  # Each tree builds into its own rust/target (env.sh), and the tree being staged is that tree, not this
  # checkout: when --repo names another tree, building into this checkout's target leaves the other tree's
  # xtask standing in this tree's build folder, and the next gate run here would run it. Staged in its own
  # tree, the release's builds join the builds that tree's own scripts make.
  export CARGO_TARGET_DIR="$repo/rust/target"
fi
# The run's folder, the tree it stages, the commit it tags and the mode it runs in are settled now, and no
# later step can move them: the clean-up reads them all, to take back the run's own tag and to end the
# run's folder as its mode promises, so a step that pointed one elsewhere would leave the run's tag
# standing, take back another's, or leave standing a folder no run said where it stands.
readonly run repo commit mode

# Each version's three numbers, or failure: a release tag is exactly X.Y.Z, and no other spelling is a
# version — no leading zeros either, which make `08` one number that tools read two ways.
X1="" X2="" X3=""
split_version() {
  if [[ $1 =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    local part
    for part in "${BASH_REMATCH[@]:1}"; do
      case "$part" in
      0 | [1-9]*) ;;
      *) return 1 ;;
      esac
    done
    X1="${BASH_REMATCH[1]}" X2="${BASH_REMATCH[2]}" X3="${BASH_REMATCH[3]}"
    return 0
  fi
  return 1
}

# The release the repository's tags hold as the newest, or none. Tags are exactly X.Y.Z (AGENTS.md §11);
# any other tag names no release, and the script has no history but these.
has_release=0
newest_major="" newest_minor="" newest_patch=""
reading_the_releases() {
  local tag
  while IFS= read -r tag; do
    split_version "$tag" || continue
    if [ "$has_release" = 0 ] || [ "$X1" -gt "$newest_major" ] ||
      { [ "$X1" -eq "$newest_major" ] && [ "$X2" -gt "$newest_minor" ]; } ||
      { [ "$X1" -eq "$newest_major" ] && [ "$X2" -eq "$newest_minor" ] && [ "$X3" -gt "$newest_patch" ]; }; then
      newest_major="$X1" newest_minor="$X2" newest_patch="$X3" has_release=1
    fi
  done < <(git -C "$repo" tag --list)
}

version=""
reading_the_version() {
  local json versions
  json="$(craftbridge_cargo_in "$repo/rust" metadata --format-version 1 --no-deps --locked)" ||
    fail "cargo metadata could not read $repo/rust"
  versions="$(printf '%s' "$json" | jq -r '[.packages[].version] | unique | join(" ")')" ||
    fail "jq could not read what cargo metadata answered"
  [ -n "$versions" ] || fail "$repo/rust answered no package to version"
  case "$versions" in
  *" "*) fail "every artifact takes one version, but the workspace's crates answer: $versions" ;;
  esac
  version="$versions"
  split_version "$version" || fail "$version is no version: tags are exactly X.Y.Z"
  # Every later step takes the version from here, and none can change it.
  readonly version
  say "rust/Cargo.toml versions every artifact $version"
}

# A version releases once, and its tag is that record. This stands before the changelog's own checks:
# a release's rows move to the version's section, so a re-release of a tagged version says the true
# reason — the tag already exists — and not "no [Unreleased] section".
refusing_a_released_version() {
  if [ "$has_release" = 1 ] && [ "$version" = "$newest_major.$newest_minor.$newest_patch" ]; then
    fail "$version is tagged already: a version releases once"
  fi
}

# Where a staging run's release stands once it has passed: the version's own folder in the repository's
# release folder, which git ignores, so a staged release is no change to the tree and stays where its
# owner publishes from until that owner moves it. A version stages once, so a folder already standing
# there is a release staged before — published or not — and this run refuses rather than mix the two.
# Named once and fixed, since the clean-up reads it to know what a failed run must not leave behind.
release_folder="release"
settling_where_the_release_stands() {
  if [ "$mode" = --stage ]; then
    staged="$repo/$release_folder/$version"
    [ ! -e "$staged" ] ||
      fail "$staged stands already: a version stages once, so publish or move that release before staging $version again"
  fi
  readonly staged
}

# The headings the rows of [Unreleased] stand under, each spelt here once: AGENTS.md §11's own heading
# for a change that breaks what a release shipped, then Keep a Changelog's. The first two decide a
# release's class.
breaking_heading="### Breaking changes"
added_heading="### Added"
headings=("$breaking_heading" "$added_heading" "### Changed" "### Deprecated" "### Removed" "### Fixed" "### Security")

unreleased=""
# The rows (`- ` items) of [Unreleased] that stand under one of its headings, or under none for "".
rows_under() {
  printf '%s\n' "$unreleased" | awk -v heading="$1" '/^### / { under = $0; next } /^- / && under == heading { n++ } END { print n + 0 }'
}
changelog_rows=0 breaking_rows=0 added_rows=0
reading_the_changelog() {
  local heading name known listed=""
  [ -f "$repo/CHANGELOG.md" ] || fail "the repository has no CHANGELOG.md: every user-visible change adds a line to it"
  grep -Eq '^## \[Unreleased\][[:space:]]*$' "$repo/CHANGELOG.md" ||
    fail "CHANGELOG.md has no [Unreleased] section: a release's rows stand there, and become the version's section"
  unreleased="$(awk '/^## \[Unreleased\][[:space:]]*$/{f=1; next} f && /^## /{f=0} f' "$repo/CHANGELOG.md")"
  changelog_rows="$(printf '%s\n' "$unreleased" | grep -c '^- ' || true)"
  [ "$changelog_rows" -gt 0 ] ||
    fail "CHANGELOG.md holds no rows under [Unreleased]: the release's rows stand there, and become the version's section"
  for name in "${headings[@]}"; do
    listed="${listed:+$listed, }$name"
  done
  while IFS= read -r heading; do
    known=0
    for name in "${headings[@]}"; do
      if [ "$heading" = "$name" ]; then known=1; fi
    done
    [ "$known" = 1 ] ||
      fail "CHANGELOG.md's [Unreleased] holds the heading \"$heading\", none of those its rows stand under ($listed): a release takes its class from them"
  done < <(printf '%s\n' "$unreleased" | grep '^###' || true)
  [ "$(rows_under "")" = 0 ] ||
    fail "CHANGELOG.md holds rows under [Unreleased] that stand under no heading: each stands under the heading of its kind, which gives a release its class"
  breaking_rows="$(rows_under "$breaking_heading")"
  added_rows="$(rows_under "$added_heading")"
  # A section for the version is the release's own section: Keep a Changelog dates one ("## [0.1.0] -
  # 2026-11-01"), so anything but whitespace after the bracketed version is that date, not a different
  # heading; any spelling of the version's own section says the version was released once.
  if grep -Eq "^## \[${version//./\\.}\]([[:space:]].*)?$" "$repo/CHANGELOG.md"; then
    fail "CHANGELOG.md has a section for $version already: those rows were released once, and a version releases once"
  fi
  say "CHANGELOG.md holds the release's $changelog_rows rows under [Unreleased]: $breaking_rows under $breaking_heading, $added_rows under $added_heading"
}

checking_the_version_against_the_releases() {
  local released next_major next_minor next_patch
  if [ "$has_release" = 0 ]; then
    [ "$version" = "0.1.0" ] || fail "the first release is 0.1.0, and this is $version"
    [ "$breaking_rows" = 0 ] ||
      fail "the first release breaks no release, since none stands before it: the rows under $breaking_heading belong under the headings of their kind"
    say "$version is the first release"
    return 0
  fi
  released="$newest_major.$newest_minor.$newest_patch"
  next_major="$((newest_major + 1)).0.0"
  next_minor="$newest_major.$((newest_minor + 1)).0"
  next_patch="$newest_major.$newest_minor.$((newest_patch + 1))"
  case "$version" in
  "$next_major")
    [ "$breaking_rows" -gt 0 ] ||
      fail "$version is a major release, and CHANGELOG.md holds no row under $breaking_heading in [Unreleased]: after $released an addition is $next_minor and a fix is $next_patch"
    ;;
  "$next_minor" | "$next_patch")
    [ "$breaking_rows" = 0 ] ||
      fail "the release's rows under $breaking_heading take the major version: after $released that is $next_major, not $version"
    [ "$version" = "$next_minor" ] || [ "$added_rows" = 0 ] ||
      fail "the release's rows under $added_heading take a minor version: after $released that is $next_minor, not $version"
    ;;
  *)
    fail "$version is no version next after $released: after it come $next_major for a breaking change, $next_minor for an addition, or $next_patch for a fix"
    ;;
  esac
  say "$version stands after the release $released"
}

# The AARs prove their own version: the publish tasks take the `craftbridge.version` property and refuse
# every publish without it, the proof repository stands inside the run's folder beside no repository
# anyone else reads, and what reaches it is read back. Gradle runs on the catalog's JDK, which the pins
# hand to JAVA_HOME.
proving_the_aar_versions() {
  local xtask_bin modules module published_name dir read_version pins
  CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR" craftbridge_cargo_in "$repo/rust" build \
    --package xtask --locked --quiet --target-dir "$run/xtask" ||
    fail "the checks could not build to read the pins Gradle runs on"
  xtask_bin="$run/xtask/debug/xtask"
  # The pins belong to the tree being released, and xtask judges the tree it runs for by where its
  # binary was built, so the pins are read from inside that tree rather than this checkout's.
  # The read is captured whole, not streamed: a pins command that fails writes no pins, and only its
  # own exit status says so — through a process substitution the status would vanish, and env.sh would
  # report a pin missing from a file that holds it, pointing the reader at the wrong file.
  pins="$(cd "$repo" && env -u CRAFTBRIDGE_ROOT "$xtask_bin" pins)" ||
    fail "the pins Gradle runs on could not be read from $repo"
  # The tool was built only to read the pins, and goes now: the run's folder is the release a staging
  # run keeps, and a build of the checks is no part of it.
  rm -rf "$run/xtask"
  # xtask refuses a pin its files lack before it prints any, so a pins command that ends on 0 has printed
  # every pin, and lines env.sh's reader cannot read are that command's fault, which env.sh would lay on a
  # file that holds the pin. So its one reader judges the lines apart first, and the release says why.
  [ -n "$pins" ] || fail "the pins command for $repo ended on 0 and printed nothing"
  (craftbridge_load_pins <<<"$pins") >/dev/null 2>&1 ||
    fail "the pins command for $repo printed no pins the build reads, beginning: $(head -n 1 <<<"$pins")"
  craftbridge_load_pins <<<"$pins"
  (
    cd "$repo/android/sdk" &&
      ./gradlew --no-daemon --warning-mode=fail --console=plain \
        publishReleasePublicationToLocalProofRepository \
        "-Pcraftbridge.version=$version" "-Pcraftbridge.localRepo=$run/maven"
  ) || fail "the AARs could not publish at $version"
  [ -d "$run/maven" ] || fail "nothing reached the proof repository: no publication published"
  modules="$(find "$run/maven" -name '*.module' -type f | sort)"
  [ -n "$modules" ] || fail "nothing reached the proof repository: no Gradle module metadata was published"
  while IFS= read -r module; do
    read_version="$(jq -r '.component.version' "$module")" || fail "jq could not read ${module#"$run/maven/"}"
    published_name="$(basename "$module" .module)"
    dir="$(dirname "$module")"
    [ -f "$dir/$published_name.aar" ] ||
      fail "${dir#"$run/maven/"} holds no ${published_name}.aar: the publication shipped no AAR to version"
    [ "$read_version" = "$version" ] ||
      fail "${module#"$run/maven/"} reads version $read_version, not the release's $version"
    say "the proof repository's ${module#"$run/maven/"} reads version $read_version, with its AAR beside it"
  done <<<"$modules"
}

# The run's own temporary folder, for the tools that leave theirs behind: SwiftPM's commands, the
# builder's among them, leave temporary folders and lock files in the one they are given and remove none
# of them, so given this one, what they leave stands in the run's folder and goes with the run, a staged
# release's included (ending_the_run_folder).
run_temporary="tmp"
with_the_run_temporary() {
  mkdir -p "$run/$run_temporary" && TMPDIR="$run/$run_temporary" "$@"
}
swift_package() {
  with_the_run_temporary swift package "$@"
}

# The distribution repository apps resolve the Swift package from (D-24): its name is the plan's, held
# here once so the folder it stands in, the address of its release and the words that send its owner to
# it all say it alike.
distribution_repository="ChristopherLinnett/SwiftEffects"
# Where the zip a package names is served: the GitHub release standing on the distribution repository's
# own tag of the version, as the distribution repositories the plan follows serve theirs
# (evidence/07-repository-conventions.md Finding 2). The package and the bytes it promises then live,
# version and stay together, and an app needs no repository but the one it resolves. Naming the place is
# all a staging run does with it: the release itself is opened by the owner, who publishes nothing less
# (plan §26, G2).
releases_at="https://github.com/$distribution_repository/releases/download"

# The XCFramework the package's binary target stands for, zipped for the release with the checksum SwiftPM
# will verify on it. The builder is the tree's own tools/build-xcframework.sh — the gate's and the
# package's one builder, never a second build: which crate is the distribution library, which targets
# it is built for and which CPU each is built at are that script's and its xtask's definitions alone
# (plan §22), so a release that joined the framework some other way would ship bytes no gate ever
# compiled. --keep-bindings because the committed bindings are part of the tree the tag versions: a
# release that regenerated them would be editing the very tree it stages.
binary_path="" binary_name="" zip_name="" zip_path="" zip_url="" zip_checksum="" distribution_tree=""
building_the_xcframework_zip() {
  local binaries manifest
  # dump-package is the official reader: the release parses no manifest text itself, and what it
  # rewrites later is anchored on what this read says, not on what it hopes a manifest looks like.
  manifest="$(swift_package dump-package --package-path "$repo/ios/sdk")" ||
    fail "swift package dump-package could not read $repo/ios/sdk"
  binaries="[$(printf '%s' "$manifest" | jq -c '.targets[] | select(.type == "binary")')]"
  # One binary target holds the release's framework: two would leave the release guessing which of
  # two the app is to download, and a source manifest already in the url form is a package released
  # before anything was built to release.
  [ "$(printf '%s' "$binaries" | jq 'length')" = 1 ] ||
    fail "$repo/ios/sdk declares no single binary target for the release's XCFramework: the release cannot say which bytes its checksum is taken of"
  binary_name="$(printf '%s' "$binaries" | jq -r '.[0].name')"
  binary_path="$(printf '%s' "$binaries" | jq -r '.[0].path // ""')"
  [ -n "$binary_path" ] ||
    fail "$repo/ios/sdk declares its binary target as a url: the source declares the framework the build joins, in the folder it joins it; the url and its checksum belong to the copy a release writes (D-24)"
  with_the_run_temporary bash "$repo/tools/build-xcframework.sh" --keep-bindings ||
    fail "the XCFramework could not be built from the tree being released"
  [ -d "$repo/ios/sdk/$binary_path" ] ||
    fail "tools/build-xcframework.sh left no $binary_path where the package's binary target reads it"
  # Named for the framework it holds and the release it belongs to, so a download, a note and a
  # checksum can all quote one name.
  zip_name="$(basename "$binary_path" .xcframework)-$version.xcframework.zip"
  zip_path="$run/$zip_name"
  # Zipped from the folder that holds the framework, so the archive opens straight to the
  # .xcframework bundle at its root, which is the shape SwiftPM unpacks; -X keeps Mac metadata out of
  # the bytes every app will one day verify one checksum against.
  (cd "$repo/ios/sdk/$(dirname "$binary_path")" && zip -q -X -r "$zip_path" "$(basename "$binary_path")") ||
    fail "the XCFramework could not be zipped into $zip_path"
  zip_checksum="$(swift_package compute-checksum "$zip_path")" ||
    fail "the checksum of $zip_path could not be computed"
  zip_url="$releases_at/$version/$zip_name"
  # The bytes, their checksum and their one address are settled here, and no later step can move them:
  # the package written next can only be written of what stands here.
  readonly binary_name binary_path zip_name zip_path zip_url zip_checksum
  say "the release's XCFramework is zipped as $zip_name with the checksum $zip_checksum"
}

# The distribution package a release writes (D-24): the tree's own ios/sdk exactly as the tagged commit
# holds it, and one difference a release must make — the binary target's local path, which points into a
# checkout, becomes the released XCFramework's url and its checksum, which is the only form an app can
# resolve: SwiftPM takes a package by URL only from a repository root, and ios/sdk is no repository root.
# It stands in the run's folder as a git tree ready to become ChristopherLinnett/SwiftEffects, whose
# first commits these very files are the owner's to push at publishing (plan §26, G2).
writing_the_distribution_package() {
  local committed files from_package manifest_without_binary products source_read to_package written written_read
  # The checksum is taken of the bytes again at the moment they are promised to the manifest: a zip that
  # changed since — by a hand, a step, a failing disk — fails here, rather than a package shipping a
  # checksum that stands for bytes no run ever read.
  [ "$(swift_package compute-checksum "$zip_path")" = "$zip_checksum" ] ||
    fail "$zip_path is no longer the bytes its checksum $zip_checksum was taken of: nothing is written that promises checksums no longer standing for them"
  distribution_tree="$run/$distribution_repository"
  mkdir -p "$distribution_tree" || fail "$distribution_tree could not be made to write the package into"
  # git's own archive writes the tagged commit's tracked files under ios/sdk — every source, manifest
  # and dotfile it ships — so the package carries the tree's Swift sources with no second list of them
  # and nothing hand-copied; what this tree holds is precisely what the tag versions. The archive's paths
  # begin ios/sdk/, and the package's manifest lies at its root (D-24), so two folders are stripped.
  git -C "$repo" archive "$commit" -- ios/sdk | tar -x --strip-components=2 -C "$distribution_tree" ||
    fail "the commit $commit could not be written out from its ios/sdk"
  from_package="$(swift_package dump-package --package-path "$repo/ios/sdk")" ||
    fail "swift package dump-package could not read $repo/ios/sdk"
  # The one edit, and only where a release must make one: the path the build joins becomes the url the
  # release serves and the checksum those bytes answer to. The line is found by the exact declaration
  # the reader reported — a manifest whose declaration this is not, in no line or in more than one, is
  # refused rather than half-written.
  if ! awk -v name="$binary_name" -v old="path: \"$binary_path\"" -v new="url: \"$zip_url\", checksum: \"$zip_checksum\"" '
    BEGIN { want = ".binaryTarget(name: \"" name "\"" }
    index($0, want) != 0 && index($0, old) != 0 {
      if (done++) { bad = 1 }
      at = index($0, old)
      print substr($0, 1, at - 1) new substr($0, at + length(old))
      next
    }
    { print }
    END { exit done == 1 && !bad ? 0 : 1 }
  ' "$distribution_tree/Package.swift" >"$distribution_tree/Package.swift.new"; then
    rm -f "$distribution_tree/Package.swift.new"
    fail "$binary_name's declaration is not the one line the manifest reader read it as: the release rewrites only that line, and writes nothing it cannot find it in"
  fi
  mv "$distribution_tree/Package.swift.new" "$distribution_tree/Package.swift" ||
    fail "the rewritten manifest could not be put in place of the source one"
  # The tree written is the commit's ios/sdk, file for file and byte for byte, the manifest's one line
  # aside: git archive answers the repository's export attributes, which can leave a file out or rewrite
  # one, so what was written is read back against the commit's own tree, through git's blob ids, before
  # any app is promised it.
  committed="$(git -C "$repo" ls-tree -r --format='%(objectname) %(path)' "$commit" -- ios/sdk | sed 's#^\([0-9a-f]*\) ios/sdk/#\1 #' | grep -v ' Package.swift$' | sort -k 2)"
  files="$(cd "$distribution_tree" && find . -type f ! -path ./Package.swift | sed 's#^\./##' | LC_ALL=C sort)"
  written="$(paste -d ' ' <(cd "$distribution_tree" && printf '%s\n' "$files" | git hash-object --no-filters --stdin-paths) <(printf '%s\n' "$files") | sort -k 2)"
  [ "$written" = "$committed" ] ||
    fail "the written package's files are not the ones $commit holds under ios/sdk: $(diff <(printf '%s\n' "$committed") <(printf '%s\n' "$written") | grep '^[<>]' | head -n 3 | tr '\n' ' ')"
  [ "$(diff <(git -C "$repo" show "$commit:ios/sdk/Package.swift") "$distribution_tree/Package.swift" | grep -c '^[<>]')" = 2 ] ||
    fail "the written Package.swift differs from the one $commit holds in more than the binary target's line"
  to_package="$(swift_package dump-package --package-path "$distribution_tree")" ||
    fail "the written Package.swift is no manifest the release reader can read"
  # Read back what was written, twice over: the binary target must answer exactly the release's url and
  # checksum and keep no path, and nothing else the manifest says may differ from the tree's own —
  # products, targets, dependencies, platforms: all of it crosses from the source with no second list,
  # and the comparison is what proves it.
  printf '%s' "$to_package" |
    jq -e --arg url "$zip_url" --arg sum "$zip_checksum" '
      [.targets[] | select(.type == "binary")] |
      length == 1 and (.[0].url == $url) and (.[0].checksum == $sum) and (.[0] | has("path") | not)' >/dev/null ||
    fail "the written package answers its binary target no release url and checksum: $(basename "$distribution_repository") would resolve nothing to download"
  manifest_without_binary='del(.packageKind) | .targets = [.targets[] | if .type == "binary" then {name, type} else . end]'
  source_read="$(printf '%s' "$from_package" | jq -S "$manifest_without_binary")" ||
    fail "jq could not reduce the source manifest to the shape the release copies"
  written_read="$(printf '%s' "$to_package" | jq -S "$manifest_without_binary")" ||
    fail "jq could not reduce the written manifest to the shape the release copies"
  [ "$source_read" = "$written_read" ] ||
    fail "the written package differs from $repo/ios/sdk beyond the binary target: it ships the tree's manifest entire, and only what a release must move, moves"
  products="$(printf '%s' "$to_package" | jq -r '[.products[].name] | join(", ")')"
  say "the ${distribution_repository##*/} package is written, ios/sdk at $commit file for file: its binary target now reads $zip_url at that checksum, and its products crossed as they stand — $products"
}

# The tag versions the Swift package's source, the commit whose ios/sdk the package is released from, so
# that commit must hold the package's manifest before it is tagged; the tag is then read back from git,
# and must name the commit that was proved.
swift_manifest="ios/sdk/Package.swift"
tagging_the_release() {
  local tagged
  [ "$(git -C "$repo" ls-tree --name-only "$commit" -- "$swift_manifest")" = "$swift_manifest" ] ||
    fail "the commit $commit holds no $swift_manifest: the tag names the commit the Swift package is released from, so it must hold the package"
  # Name the tag before making it, so a run stopped in the moment between the two still has a cleanup
  # that knows which tag to take back: bash hands a pending signal to the exit trap between commands,
  # and there the tag would exist while the cleanup still read none.
  tag_made="$version"
  # git refuses a tag that already stands, and a tag this run could not make is no tag of its own, even on
  # its commit: the run holds the repository's release lock whole, so a tag standing here was made beside
  # it — by a hand, or left by a run killed outright — and is no tag of this run's to take back.
  git -C "$repo" tag "$version" "$commit" || {
    tag_made=""
    fail "git could not tag $commit $version"
  }
  tagged="$(git -C "$repo" rev-parse -q --verify "refs/tags/$version^{commit}")" || fail "git holds no tag $version"
  [ "$tagged" = "$commit" ] || fail "the tag $version names $tagged, not the commit $commit the run proved"
  say "tagged $version, read back from git: the commit $commit, whose tree holds the Swift package's source, $swift_manifest"
}

saying_what_was_staged() {
  say "every artifact this stage makes reads $version: the crates of rust/Cargo.toml, the AARs the proof repository holds, the XCFramework zip and the release url the ${distribution_repository##*/} package names it by, and the tag on the Swift package's source"
  if [ "$mode" = --dry-run ]; then
    say "the rehearsal wrote only its own folder, which goes now with its clone, its tag, its zip and its written package: nothing of $src was written"
  else
    say "the tag $version stands in $src, unpushed: nothing was pushed, no release was made and no package was uploaded, which the owner decides"
    say "the release stands staged in $staged as the run ends, for its owner to publish from: the ${distribution_repository##*/} tree in ${distribution_tree#"$run/"}, the zip ${zip_name} its release serves at $zip_url, and the proof repository in maven"
  fi
}

# The release, step by step. A later stage joins this list before the tag, which stays the last write.
reading_the_releases
reading_the_version
refusing_a_released_version
settling_where_the_release_stands
reading_the_changelog
checking_the_version_against_the_releases
proving_the_aar_versions
building_the_xcframework_zip
writing_the_distribution_package
tagging_the_release
saying_what_was_staged
# The run passes here and ends here, with nothing between: bash 3.2 hands the exit trap a 0 for a
# statement that meets an unset variable, and a signal hands it the status of the command before, so a
# statement after the pass mark could fail, or be stopped, and the trap still read a pass and keep the tag.
craftbridge_passed=1
exit 0
