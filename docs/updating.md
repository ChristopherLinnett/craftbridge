# Updating EffectCraft

EffectCraft is upstream: this SDK builds official `storytold/effectcraft` revisions exactly as they are (AGENTS §7). The pin moves only through one script, and only to a commit official main reaches. This page is the maintainer's route through one update; the full design lives in the plan of record, §24.

`tools/update-upstream.sh <upstream-commit>` runs the whole update, its seven steps in one run, and leaves a person what only a person can do: recheck what it lists, and finish the record it writes.

1. **Fetch.** The script fetches official main into the machine's read-only cache of it when the cache does not reach the commit yet, and refuses a branch, a tag, or a commit official main does not reach.
2. **Pins.** The script writes the new `rev`, EffectCraft's FilmCraft revision, and each Craft library pin — each taken from what the new EffectCraft itself pins — and copies the `[patch.crates-io]`, `[profile.dev]` and `[profile.release]` tables of `rust/Cargo.toml` from upstream's manifest. `Cargo.lock` starts from upstream's and CraftBridge's own crates resolve against it. These tables and the starting lock are upstream's and are never hand-edited; a pin, a `[patch]` table or a profile that moves while the lock stays is exactly the drift the gate hunts.
3. **Adaptations.** Every live entry of `tools/adaptations.toml` lists the EffectCraft files and items it relies on; the run diffs them between the old and new revision and names each adaptation whose ground moved, with its diff, for you to recheck against upstream's new code. Retired entries — ones upstream later replaced with a public interface — are listed apart, each naming the commit that replaced it, and are not rechecked. If upstream's new interface does an adaptation's job, the adaptation moves onto it and is retired in the list, not deleted.
4. **Regenerate.** The run writes everything `cargo xtask generate` writes — the API surface first, then what is made from it: both facade trees, the version constants, and `docs/generated/commands.md`, the command reference, rendered from the same surface file by the same codegen (never written by hand; the gate's `generated` stage fails any byte of it that differs) — and diffs the new surface against the one the tree held. A lost command or a key that turned required breaks released apps: the next release is a major version, a conversation to have before merge.
5. **Gate the parts an update moves** — `tools/ci.sh rules duplicates conformance rust_mobile`: the pin and duplicate-crate rules over the new lock, the desktop conformance corpus against the new revision's command line, and the mobile type-checks. The whole gate stays hosted CI's (AGENTS §12).
6. **Record.** The run appends its entry to [upstream-updates.md](upstream-updates.md): from and to revisions, the gate's result, the commands added, changed and removed, and a line for each adaptation it named, which you complete with how its recheck resolved. The log is the memory of how each pin was taken.
7. **Versions.** The run names every library in `android/sdk/gradle/libs.versions.toml` and the Gradle wrapper whose newer release is out, for a person to take, each in a change of its own — this is where library versions are checked, since the gate's Lint deliberately ignores daily newer versions.

## States an update can end in

- **Up to date** — the named revision is the pin; the run stops after step 2, and its last line says nothing changed.
- **Updated** — the run exits 0, cleanly or with adaptations to recheck; once each is rechecked and its record line completed, the change is ready for its pull request.
- **Breaking API change** — the run exits 2; the next release is a major version.
- **Gate failure** — the run exits 1, its output naming the failing stages and scenarios, with the new pin in the tree.

## Rules that hold it all together

- Never a fork, never a patch branch, never an opened upstream issue: adaptations live on this side, written against EffectCraft's public interfaces (AGENTS §7).
- The pin is one commit of official main, named by its id — never a branch, tag or PR head.
- One copy of each EffectCraft, FilmCraft and Craft library crate; the duplicate-crate check inside `rules` proves it over the new lock.
- Every GitHub call an updater makes names the `ChristopherLinnett` account explicitly, because a machine's active login may be another account.
- The guides' code is compiled quotation: when a regenerated API changes what the samples do, the samples change first and the guides follow them (`cargo xtask guide-samples` refuses a guide block that is no run of a file the samples compile).
