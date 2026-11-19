//! The iPhone simulator the Swift Testing suite runs on: `cargo xtask simulator -- <program> <argument>…`
//! runs the program with every `{simulator}` in its arguments replaced by the id of one of the machine's
//! own iPhone simulators, and exits as the program does.
//!
//! The simulators are the machine's, shared by every run on it — another worktree's gate, a developer's or
//! an agent's own test run — so nothing here creates, deletes, erases or renames one. A run takes one that
//! is running already when one is free, and boots one only when none is: the newest iOS runtime's, so the
//! choice never rests on the order `simctl` happens to list its runtimes in. It shuts down only a
//! simulator whose boot a run of this command caused, itself or through its program, and only once nothing uses it: no run of this command holds
//! it, no run started without a lock names it in its arguments, and no package's tests run in it,
//! whoever started them. The same look, made when a run chooses, keeps it off a simulator a run outside
//! the lock is using: such a run boots its destination itself and shuts down what it booted when it
//! ends, and nothing holding a lock can wait that out. The run says which it did and why: it takes
//! another available iPhone of the used one's runtime, booting it under its own lock as it boots one it
//! found stopped, or, when the machine has no other of that runtime free, waits for the run using it to
//! end and chooses again.
//!
//! Runs of this command take turns through the system's file locks in `CRAFTBRIDGE_SIMULATOR_LOCKS`
//! (`tools/env.sh`), which the system lets go however a run ends. A run holds one simulator's lock whole
//! while its program runs on it, so two runs never test on one simulator together, and the run on it
//! decides alone whether to shut down one it booted: a program may write into the simulator's own data
//! folder, which is every run's on that simulator — a Swift package's tests do, as their app sandbox,
//! unless given one of their own, as the `swift_test` stage gives them — and the engine's export journal
//! there lets one process hold it at a time. A run that finds its chosen simulator held waits for its turn; what it looked at before waiting is
//! stale once the lock is its own — the simulator may have been booted, tested on and shut down meanwhile
//! — so it looks at the state again under the lock, and boots a simulator it finds stopped itself.
//! Under the lock no other run of this command boots the simulator or shuts it down, so every boot it has
//! while the run holds it is the runs' but one: the boot that was running already when the run looked,
//! which somebody else made and is left to them. A boot made since is the run's own — its `simctl`'s, which
//! boots the simulator again if it went down between the look and the wait for its boot, or its program's
//! — or one made by whoever booted the simulator again after the boot it found had ended, and so the
//! run's as one it found stopped. The same lock file says, with the boot `simctl` records
//! (`lastBootedAt`), that a run of this command caused the boot the simulator has: the run holding the
//! lock at the shutdown decision shuts it down, and a boot a run left running for another's use is the
//! runs' when the next run finds that very boot. A run that takes no lock — `xcodebuild` started by hand,
//! which boots its destination itself, and shuts down when it ends a simulator it booted — shows twice
//! over, and one look reads it at both the decisions that need it: whether to take the simulator, and
//! whether to shut it down.
//! While its tests run they show in the simulator: a Swift package's tests run in Xcode's test runner,
//! `xctest`, a job of the device's own launchd. And from the moment it starts it shows in the machine's
//! processes, because such a run carries the simulator's id in its own arguments — what
//! `xcodebuild -destination id=` and `simctl` are started with — which the look asks `pgrep` about,
//! saving the device's own `launchd_sim`, which names it merely by the device being up. A run that names
//! the simulator by id only after the look is seen too: if its boot beats this run's — which only a run
//! outside the lock can do — the run chooses again from the look, which then sees it, and if this run's
//! boot is first, that run never shuts the simulator down, since `xcodebuild` shuts down only a simulator
//! it booted. What no look can see is a run naming its destination by device name, not id, outside its tests:
//! before they start, and from their end until its `xcodebuild` shuts down what it booted. The name
//! belongs to no one simulator, so no rule can tell that such a run would take this one, or has; a run
//! hidden so, ending and shutting down what it booted under a suite already running, fails that suite —
//! the harm this look narrows rather than closes.
//!
//! What the machine holds is read from `simctl`'s own JSON (`xcrun simctl list --json`): which runtimes are
//! iOS's and available, which device types are iPhones (the runtime's own `productFamily`), and each
//! device's state. `launchctl list` has no other form than its table of jobs, a pid or `-`, a status and a
//! label, which is what is read of it.

use serde_json::Value;
use std::fs::File;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

/// What a program's arguments say where the simulator's id goes.
const PLACEHOLDER: &str = "{simulator}";

/// The tool that answers whether any process on the machine matches a pattern in its arguments.
const PGREP: &str = "pgrep";

/// The launchd daemon a booted simulator runs its jobs through, named by no run: it carries its device's
/// id in its own arguments for as long as the device is booted, so it says the device is up, not used.
const LAUNCHD: &str = "launchd_sim";

/// What the machine is told when it holds no iPhone simulator the suite could run on: `where_to_run`
/// says it, finding nothing to take and no run to wait for.
const NO_IPHONE: &str = "no iPhone simulator is available to run the Swift suite on: install the runtime with xcodebuild -downloadPlatform iOS and create a device with xcrun simctl create";

/// What a simulator's lock file holds while its boot, which `simctl` records at `at`, is one a run of this
/// command caused. The words are the file's, read back by `boot_of_a_run`.
fn booted_by_a_run(at: &str) -> String {
    format!("booted at {at}\n")
}

/// The boot a lock file's words say a run of this command caused, `None` when they say none.
fn boot_of_a_run(said: &str) -> Option<&str> {
    said.strip_prefix("booted at ")?.strip_suffix('\n')
}

/// The boot somebody else made: `running`, the boot the simulator had when a run holding its lock looked at
/// it, unless a run of this command caused that boot. A run
/// did when the simulator was stopped and the run booted it (`running` is `None`), and when `said`, the
/// lock file, names that very boot as one a run left running for another's use.
fn theirs<'a>(running: Option<&'a str>, said: &str) -> Option<&'a str> {
    running.filter(|at| boot_of_a_run(said) != Some(*at))
}

/// What the lock file says while the simulator runs at `at` under a run that found `theirs` running: a
/// boot a run caused, unless it is that one. A boot other than the one the run found is one made while
/// the run held the lock, which no other run of this command can make — so it is not compared with the
/// boot the run itself made, and the run's own `simctl` or program booting the simulator again keeps it the
/// runs'.
fn said_while_running(theirs: Option<&str>, at: &str) -> String {
    if theirs == Some(at) { String::new() } else { booted_by_a_run(at) }
}

/// One of the machine's iPhone simulators, as `simctl` lists it.
#[derive(Clone, Debug, PartialEq)]
struct Device {
    udid: String,
    name: String,
    running: bool,
}

/// `cargo xtask simulator -- <program> <argument>…`.
pub(crate) fn run(arguments: &[&str]) -> ExitCode {
    match naming_the_simulator(arguments).and_then(|(program, arguments)| on_simulator(program, arguments)) {
        Ok(exit) => exit,
        Err(failure) => crate::report::failed("simulator", failure),
    }
}

/// The program and its arguments, refused unless an argument names the simulator: a program that names it
/// nowhere would run wherever it chose, beside the simulator this run holds and the others take turns on.
fn naming_the_simulator<'a>(arguments: &'a [&'a str]) -> Result<(&'a str, &'a [&'a str]), String> {
    arguments
        .split_first()
        .filter(|(_, arguments)| arguments.iter().any(|argument| argument.contains(PLACEHOLDER)))
        .map(|(program, arguments)| (*program, arguments))
        .ok_or_else(|| format!("names no program to run with {PLACEHOLDER} where the simulator's id goes: cargo xtask simulator -- <program> <argument>…"))
}

/// Take a simulator, run the program on it, and leave the simulator as the runs on the machine need it.
/// The simulator is left behind even when it did not finish booting or the program could not start, so a
/// run that booted one never leaves it running for want of a program.
fn on_simulator(program: &str, arguments: &[&str]) -> Result<ExitCode, String> {
    let locks = crate::filelock::folder()?;
    std::fs::create_dir_all(&locks).map_err(|error| format!("could not create {}: {error}", locks.display()))?;
    let (device, theirs, using) = take(&locks)?;
    let marker = crate::filelock::for_id(&locks, &device.udid);
    let ran = ready(&marker, &device, theirs.as_deref()).and_then(|()| {
        println!("simulator: the suite runs on {} ({})", device.name, device.udid);
        Command::new(program).args(with_simulator(arguments, &device.udid)).status().map_err(|error| format!("could not run {program}: {error}"))
    });
    let left = leave(&marker, &device, theirs.as_deref(), using);
    let status = ran?;
    left?;
    Ok(crate::report::exit_of("simulator", program, status))
}

/// The simulator to run on, held whole so that this run is alone on it: `where_to_run` says which the
/// listing offers and what to make of a simulator a run outside the lock is using — another of its
/// runtime to boot, or the wait for that run to end — and the simulator's lock, which waits out any run
/// whose program is still testing, says when it is this run's turn. Another run may have booted, tested
/// on and shut down a listed running simulator while this one looked or waited, so under the lock the
/// state is looked at again: a simulator a run outside the lock has taken in the meantime is passed over
/// for that look to place, and a simulator found stopped is booted here, which no other run of this
/// command can be doing at the same time because it is this run that holds the simulator's lock — so the
/// boot is this run's to record, and to undo, and a boot that beats it is a run's outside the lock, whose
/// simulator is passed over the same way. Taken with it is the boot somebody else had made, running when
/// this run looked, which stays theirs.
fn take(locks: &Path) -> Result<(Device, Option<String>, File), String> {
    // The one look of a run outside the lock using a simulator, the one `leave` reads too: a run of this
    // command is never on a simulator without holding its lock, so while the lock is held the simulator is
    // another run's own, waited for in the lock itself, and this answers nothing of it.
    let used = |device: &Device| -> Result<Option<String>, String> {
        if crate::filelock::held(&crate::filelock::for_id(locks, &device.udid))? {
            return Ok(None);
        }
        used_outside_the_lock(device, device.running)
    };
    loop {
        let device = match where_to_run(&iphones(&listing()?), &used)? {
            Choice::Take { device, instead_of: Some((busy, because)) } => {
                println!("{}", said_taking_instead(&device, &busy, &because));
                device
            }
            Choice::Take { device, instead_of: None } => device,
            Choice::Wait { device, because } => {
                println!("{}", said_waiting(&device, &because));
                wait_until_unused(&device)?;
                continue;
            }
        };
        let marker = crate::filelock::for_id(locks, &device.udid);
        if crate::filelock::held(&marker)? {
            println!("simulator: the suite waits for {} ({}): another run is testing on it", device.name, device.udid);
        }
        let using = crate::filelock::hold(&marker)?;
        // Passed over, the run lets the lock go as this turn of the loop ends, and starts again from the look.
        let Held::Taken(running) = under_the_lock(&listing()?, &device, &used_outside_the_lock, &boot)? else { continue };
        let said = std::fs::read_to_string(&marker).unwrap_or_default();
        return Ok((device, theirs(running.as_deref(), &said).map(str::to_string), using));
    }
}

/// What a run makes of the simulator whose lock it has just taken.
#[derive(Debug, PartialEq)]
enum Held {
    /// Passed over, for the look to place: gone, used from outside the lock, or booted by a run outside
    /// the lock as this run booted it.
    PassedOver,
    /// Taken, with the boot it had when the run looked: `None` when the run booted it itself.
    Taken(Option<String>),
}

/// What a run that has just taken a simulator's lock makes of it, from `listed`, the listing read under the
/// lock, `used`, the one look of a run outside the lock, and `boot`, which says whether this run's own boot
/// was the one that took. Under the lock the look is exact: no run of this command is on the simulator
/// without its lock, which this run holds. A run outside the lock may have chosen the simulator since the
/// look that chose it, and its own end would shut the simulator down under tests that cannot stop it, so
/// such a simulator is passed over. A simulator found stopped is booted here; one somebody else boots as
/// this run does is passed over too, since only a run outside the lock can — an `xcodebuild` booting its
/// destination, which shuts it down when it ends — and the look that places it again sees one that names
/// the simulator by id in its arguments.
fn under_the_lock(
    listed: &Value,
    device: &Device,
    used: &dyn Fn(&Device, bool) -> Result<Option<String>, String>,
    boot: &dyn Fn(&Device) -> Result<bool, String>,
) -> Result<Held, String> {
    if simulator(listed, &device.udid).is_none() {
        return Ok(Held::PassedOver); // the simulator is gone, its runtime deleted while this run waited for it
    }
    let running = booted_at(listed, &device.udid);
    if used(device, running.is_some())?.is_some() {
        return Ok(Held::PassedOver);
    }
    if running.is_some() {
        return Ok(Held::Taken(running));
    }
    Ok(if boot(device)? { Held::Taken(None) } else { Held::PassedOver })
}

/// Wait for the run outside the lock using the simulator to end: such a run took no lock, so nothing can
/// make it wait its turn, and its own end frees the simulator and shuts down what it booted there. The
/// machine is looked at again every second, since between looks the simulator may have been shut down,
/// when only the processes naming it are left to wait out. The wait has no bound, as the wait for a
/// simulator's lock has none: a run by hand lasts as long as whoever started it lets it, and a run that
/// gave up on it would fail the gate for a run by hand, which says nothing about the code. The line said
/// before the wait names what it waits out.
fn wait_until_unused(device: &Device) -> Result<(), String> {
    let mut look = || used_outside_the_lock(device, booted_at(&listing()?, &device.udid).is_some());
    let mut pause = || std::thread::sleep(std::time::Duration::from_secs(1));
    until_unused(&mut look, &mut pause)
}

/// The wait itself: `look`, the one look at the simulator as it is now, again after each `pause`, until it
/// clears the simulator. A look that could not answer ends the wait with its failure.
fn until_unused(look: &mut dyn FnMut() -> Result<Option<String>, String>, pause: &mut dyn FnMut()) -> Result<(), String> {
    while look()?.is_some() {
        pause();
    }
    Ok(())
}

/// Make the taken simulator ready for the program. The lock file says whose its boot is as soon as it runs,
/// since a run killed while it waits for the boot never lets go of the simulator, and leaves it running with
/// only the file to tell the next run it is the runs'; and again once it has finished booting, since
/// `simctl bootstatus -b` boots the simulator again if it went down meanwhile.
fn ready(marker: &Path, device: &Device, theirs: Option<&str>) -> Result<(), String> {
    record(marker, device, theirs)?;
    wait_until_booted(device)?;
    record(marker, device, theirs)
}

/// Say in the simulator's lock file whose the boot it has now is, when it is running.
fn record(marker: &Path, device: &Device, theirs: Option<&str>) -> Result<(), String> {
    match booted_at(&listing()?, &device.udid) {
        Some(at) => say(marker, &said_while_running(theirs, &at)),
        None => Ok(()),
    }
}

/// Write the simulator's lock file.
fn say(marker: &Path, words: &str) -> Result<(), String> {
    std::fs::write(marker, words).map_err(|error| format!("could not write {}: {error}", marker.display()))
}

/// The program's arguments with the simulator's id where each says.
fn with_simulator(arguments: &[&str], udid: &str) -> Vec<String> {
    arguments.iter().map(|argument| argument.replace(PLACEHOLDER, udid)).collect()
}

/// `simctl`'s whole listing, as its JSON.
fn listing() -> Result<Value, String> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "--json"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not run xcrun simctl list: {error}"))?;
    if !output.status.success() {
        return Err(format!("xcrun simctl list --json failed ({})", output.status));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("xcrun simctl list --json printed no JSON: {error}"))
}

/// The machine's iPhone simulators in the order this run chooses among them: the newest available iOS
/// runtime's first, and `simctl`'s own order within a runtime. Only iPhones, as their runtime names its
/// device types: the suite is the iPhone's (plan §28). Only what `simctl` marks available, which leaves
/// out a device whose runtime is gone, or one this Xcode cannot run. `where_to_run` takes from this list
/// and says what the machine lacks when it is empty.
fn iphones(listing: &Value) -> Vec<(Vec<u64>, Device)> {
    let devices = listing.get("devices").and_then(Value::as_object);
    let mut found: Vec<(Vec<u64>, Device)> = Vec::new();
    for runtime in listing.get("runtimes").and_then(Value::as_array).into_iter().flatten() {
        if runtime.get("platform").and_then(Value::as_str) != Some("iOS") || runtime.get("isAvailable").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(identifier) = runtime.get("identifier").and_then(Value::as_str) else { continue };
        let version: Vec<u64> = runtime.get("version").and_then(Value::as_str).unwrap_or_default().split('.').map(|part| part.parse().unwrap_or(0)).collect();
        let iphones: Vec<&str> = runtime
            .get("supportedDeviceTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|kind| kind.get("productFamily").and_then(Value::as_str) == Some("iPhone"))
            .filter_map(|kind| kind.get("identifier").and_then(Value::as_str))
            .collect();
        for device in devices.and_then(|devices| devices.get(identifier)).and_then(Value::as_array).into_iter().flatten() {
            let kind = device.get("deviceTypeIdentifier").and_then(Value::as_str).unwrap_or_default();
            if device.get("isAvailable").and_then(Value::as_bool) != Some(true) || !iphones.contains(&kind) {
                continue;
            }
            let (Some(udid), Some(name), Some(state)) =
                (device.get("udid").and_then(Value::as_str), device.get("name").and_then(Value::as_str), device.get("state").and_then(Value::as_str))
            else {
                continue;
            };
            found.push((version.clone(), Device { udid: udid.to_string(), name: name.to_string(), running: matches!(state, "Booted" | "Booting") }));
        }
    }
    // Newest runtime first; the sort is stable, so within a runtime `simctl`'s own order stands.
    found.sort_by(|one, other| other.0.cmp(&one.0));
    found
}

/// Where this run runs: a simulator to take now, or one to wait for.
#[derive(Debug)]
enum Choice {
    /// The simulator to take now. `instead_of` names the running simulator this run passed over because a
    /// run outside the lock was using it, with the words that say so — `None` when it passed nothing over.
    Take { device: Device, instead_of: Option<(Device, String)> },
    /// The simulator whose run outside the lock this run waits out before it chooses again: every
    /// simulator this run could take is used from outside the lock, and none of the runtime it wanted most
    /// has another free to boot. `because` says what uses it.
    Wait { device: Device, because: String },
}

/// Where this run runs among `iphones`' ordered simulators, as `used` — the one look of a run outside
/// the lock using one — says each stands: the first running simulator `used` clears, so a running one is
/// taken before a second is booted beside it; when every running one is used, the first available
/// simulator of the first of theirs that `used` clears, to boot under this run's lock — a simulator of
/// another runtime is not booted while one of that runtime may free, because that runtime is where this
/// machine chose to run already; and with none of that runtime free, the run waits for the run using the
/// first to end. With nothing running, the first simulator `used` clears is booted, since a run outside
/// the lock may already have named a stopped one as its destination and will boot it; when every
/// simulator is used, the run waits for the run using the one the newest runtime offers first.
fn where_to_run(found: &[(Vec<u64>, Device)], used: &dyn Fn(&Device) -> Result<Option<String>, String>) -> Result<Choice, String> {
    let mut passed: Option<((Vec<u64>, Device), String)> = None;
    for candidate in found.iter().filter(|(_, device)| device.running) {
        match used(&candidate.1)? {
            None => return Ok(Choice::Take { device: candidate.1.clone(), instead_of: None }),
            Some(because) => {
                if passed.is_none() {
                    passed = Some((candidate.clone(), because));
                }
            }
        }
    }
    if let Some(((version, busy), because)) = passed {
        for (_, other) in found.iter().filter(|(v, device)| **v == version && !device.running) {
            if used(other)?.is_none() {
                return Ok(Choice::Take { device: other.clone(), instead_of: Some((busy, because)) });
            }
        }
        return Ok(Choice::Wait { device: busy, because });
    }
    let mut waiting: Option<(Device, String)> = None;
    for (_, device) in found.iter() {
        match used(device)? {
            None => return Ok(Choice::Take { device: device.clone(), instead_of: None }),
            Some(because) => {
                if waiting.is_none() {
                    waiting = Some((device.clone(), because));
                }
            }
        }
    }
    match waiting {
        Some((device, because)) => Ok(Choice::Wait { device, because }),
        None => Err(NO_IPHONE.to_string()),
    }
}

/// What the run says when it takes another simulator in place of the one a run outside the lock is using.
fn said_taking_instead(device: &Device, instead_of: &Device, because: &str) -> String {
    format!("simulator: the suite takes {} ({}) instead of {} ({}): {because}", device.name, device.udid, instead_of.name, instead_of.udid)
}

/// What the run says when it waits for the run outside the lock using a simulator to end, the machine
/// having no other of that runtime free to boot. Once that run ends the run chooses again, since its end may
/// have shut the simulator down, and the choice with nothing running takes the newest runtime's first.
fn said_waiting(device: &Device, because: &str) -> String {
    format!(
        "simulator: the suite waits for {} ({}): {because}; no other iPhone of its runtime is free to boot, so it chooses again once that run ends",
        device.name, device.udid
    )
}

/// The device with this id among those the listing lists, as `simctl` listed it: `None` when it lists
/// none, a runtime deleted between the look and now.
fn simulator<'a>(listing: &'a Value, udid: &str) -> Option<&'a Value> {
    listing
        .get("devices")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|devices| devices.values())
        .filter_map(Value::as_array)
        .flatten()
        .find(|device| device.get("udid").and_then(Value::as_str) == Some(udid))
}

/// When the simulator with this id was booted, as the listing records it, while it is running: `None`
/// when it is not.
fn booted_at(listing: &Value, udid: &str) -> Option<String> {
    simulator(listing, udid)
        .filter(|device| matches!(device.get("state").and_then(Value::as_str), Some("Booted" | "Booting")))
        .map(|device| device.get("lastBootedAt").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// Boot the simulator: `true` when this run booted it, and `false` when another run booted it first, which
/// is no failure of the boot, since the simulator runs. Only a run that takes no lock can do that, an
/// `xcodebuild` booting its destination, so that boot is that run's.
fn boot(device: &Device) -> Result<bool, String> {
    let output = Command::new("xcrun").args(["simctl", "boot", &device.udid]).output().map_err(|error| format!("could not run xcrun simctl boot: {error}"))?;
    if output.status.success() {
        return Ok(true);
    }
    if booted_at(&listing()?, &device.udid).is_some() {
        return Ok(false);
    }
    Err(format!("the iPhone simulator {} ({}) would not boot: {}", device.name, device.udid, String::from_utf8_lossy(&output.stderr).trim()))
}

/// Wait until the simulator has finished booting: a test run on one still starting fails.
fn wait_until_booted(device: &Device) -> Result<(), String> {
    let status = Command::new("xcrun")
        .args(["simctl", "bootstatus", &device.udid, "-b"])
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run xcrun simctl bootstatus: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("the iPhone simulator {} ({}) did not finish booting", device.name, device.udid)) }
}

/// Let go of the simulator, and shut it down when its boot is the runs' — any but `theirs`, the boot
/// somebody else had made when this run looked — and nothing uses it: no run started without a lock names
/// it in its arguments, and no test of anybody's runs in it. This run has held the simulator's lock whole
/// since it took it — `_using` is that hold, dropped only as this ends — so no run of this command is on
/// the simulator through these looks and the shutdown, and a run waiting for its turn takes the lock only
/// once this has shut the simulator down or decided to leave it running, with the file saying whose boot
/// it left.
fn leave(marker: &Path, device: &Device, theirs: Option<&str>, _using: File) -> Result<(), String> {
    let Some(at) = booted_at(&listing()?, &device.udid) else {
        println!("simulator: {} is shut down already: something else shut it down", device.name);
        return Ok(());
    };
    let said = said_while_running(theirs, &at);
    say(marker, &said)?;
    if boot_of_a_run(&said).is_none() {
        println!("simulator: {} is left running, as it was found: no run of the suite booted it", device.name);
        return Ok(());
    }
    // The one look of a run outside the lock using the simulator, in its one words — the same look and
    // words `take` reads before choosing a simulator. The simulator runs — `booted_at` above said so —
    // so the tests in it are asked about too.
    if let Some(used) = used_outside_the_lock(device, true)? {
        println!("simulator: {} is left running: {used}", device.name);
        return Ok(());
    }
    let shutdown =
        Command::new("xcrun").args(["simctl", "shutdown", &device.udid]).status().map_err(|error| format!("could not run xcrun simctl shutdown: {error}"))?;
    if !shutdown.success() {
        return Err(format!("the iPhone simulator {} ({}), which a run of the suite booted, would not shut down", device.name, device.udid));
    }
    println!("simulator: {} shut down: a run of the suite booted it, and nothing uses it now", device.name);
    Ok(())
}

/// Whether a run outside the lock is using the simulator, and the words that say how: a process that is
/// not the device's own `launchd_sim` names it in its arguments, or Xcode's test runner runs in it. This
/// is the one definition here of a run outside the gate using a simulator, in its one wording, read at
/// both decisions that turn on it: `leave` refuses to shut such a simulator down, and `where_to_run`
/// refuses to take one — such a run boots its destination itself and shuts down what it booted when it
/// ends, and nothing holding a lock can wait that out. `running` says whether the simulator runs.
fn used_outside_the_lock(device: &Device, running: bool) -> Result<Option<String>, String> {
    in_use(&named_processes(&device.udid)?, running, || tests_running(device))
}

/// What the two looks say together: `users`, the processes naming the simulator, show a run started by id
/// from its first moment; failing them, `tests`, the test jobs in the simulator, show one started by name
/// once its tests run. The tests are asked of a running simulator alone, since a test runs in it only while
/// it does and the launchd of one shut down answers nothing, and only when no process names it already. A
/// look that could not answer is no "unused": it stops the decision.
fn in_use(users: &[String], running: bool, tests: impl FnOnce() -> Result<Vec<String>, String>) -> Result<Option<String>, String> {
    if !users.is_empty() {
        return Ok(Some(used_by_processes(users)));
    }
    if !running {
        return Ok(None);
    }
    let tests = tests()?;
    Ok((!tests.is_empty()).then(|| used_by_tests(&tests)))
}

/// The words for processes naming the simulator in their arguments: a run started by hand that carries
/// the id it will boot and shut down, from its first moment.
fn used_by_processes(users: &[String]) -> String {
    format!("a run started without a lock names it in its arguments (pid {})", users.join(", "))
}

/// The words for test jobs running in the simulator: a hand run the id alone never showed, whose test has
/// now started inside it.
fn used_by_tests(tests: &[String]) -> String {
    format!("a test of another run runs in it ({})", tests.join(", "))
}

/// The tests running in the simulator, by their launchd jobs. A simulator seen running may have gone down
/// since, shut down by whoever booted it — an `xcodebuild` ending does — and its launchd answers nothing
/// while it shuts down or once it has: no test runs in it then, which is all this asks, so that answer is
/// no failure of the look.
fn tests_running(device: &Device) -> Result<Vec<String>, String> {
    let output = Command::new("xcrun")
        .args(["simctl", "spawn", &device.udid, "launchctl", "list"])
        .output()
        .map_err(|error| format!("could not ask the iPhone simulator {} ({}) what it runs: {error}", device.name, device.udid))?;
    if output.status.success() {
        return Ok(test_jobs(&String::from_utf8_lossy(&output.stdout)));
    }
    if booted_at(&listing()?, &device.udid).is_none() {
        return Ok(Vec::new());
    }
    Err(format!(
        "could not ask the iPhone simulator {} ({}) what it runs: launchctl list failed ({}): {}",
        device.name,
        device.udid,
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// Which processes of this machine name the simulator in their arguments: a run started without a lock —
/// an `xcodebuild -destination id=` or a `simctl` aimed at the device, started by hand — carries its id
/// from the moment it starts, while inside the device it shows only once its tests run. A decision to
/// shut down waits until none does, save the device's own launchd daemon, which names it merely by being
/// booted. The id is read in any case, because `simctl` takes an id in any case: a run that spells it
/// lower names this very simulator to the machine, and a look blind to that spelling would boot and shut
/// down under it blind too.
fn named_processes(udid: &str) -> Result<Vec<String>, String> {
    let found = pgrep(&["-f", "-i", udid]).ok_or_else(|| format!("{PGREP} could not say which processes name the simulator {udid}"))?;
    let daemons = pgrep(&["-x", LAUNCHD]).ok_or_else(|| format!("{PGREP} could not say which processes are a simulator's own launchd"))?;
    Ok(without_launchd(&found, &daemons))
}

/// The pids `pgrep` printed for these arguments: `None` when it could not answer, which is no "none".
fn pgrep(arguments: &[&str]) -> Option<Vec<String>> {
    let found = Command::new(PGREP).args(arguments).output().ok()?;
    named_pids(found.status.code(), &String::from_utf8_lossy(&found.stdout))
}

/// The processes naming the simulator, without its own launchd daemon: `launchd_sim` runs beside a
/// booted simulator as the device's own job — never as a run started with it — so its naming the device
/// says the device is up, not that something uses it, and it is no reason to leave it running.
fn without_launchd(users: &[String], launchds: &[String]) -> Vec<String> {
    users.iter().filter(|pid| !launchds.contains(pid)).cloned().collect()
}

/// The pids `pgrep -f` named from its answer: the pids it printed when it found processes (exit 0), none
/// when it found none (exit 1), and no answer at all when it failed or was itself ended — which is no
/// "none", and stops the decision rather than shutting a simulator down under something unnamed.
fn named_pids(exit: Option<i32>, listing: &str) -> Option<Vec<String>> {
    match exit {
        Some(0) => Some(listing.lines().filter(|line| !line.trim().is_empty()).map(|line| line.trim().to_string()).collect()),
        Some(1) => Some(Vec::new()),
        _ => None,
    }
}

/// The labels of the running test jobs in a `launchctl list` table: a job with a pid, not `-`, whose label
/// is Xcode's test runner, `xctest`, in which a Swift package's tests run on a simulator, as a job of the
/// device's own launchd (`com.apple.xpc.launchd.oneshot.0x10000001.xctest`).
fn test_jobs(table: &str) -> Vec<String> {
    table
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let (pid, _status, label) = (columns.next()?, columns.next()?, columns.next()?);
            (pid.parse::<u32>().is_ok() && label.rsplit('.').next() == Some("xctest")).then(|| label.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A machine with two iOS runtimes and one it cannot run, an iPad and a watch beside the iPhones, and a
    /// device whose runtime is gone: what `xcrun simctl list --json` holds, cut to what is read.
    fn machine(booted: &[&str]) -> Value {
        let state = |udid: &str| if booted.contains(&udid) { "Booted" } else { "Shutdown" };
        let iphone = |name: &str| json!({ "identifier": format!("com.apple.CoreSimulator.SimDeviceType.{name}"), "productFamily": "iPhone" });
        let ipad = json!({ "identifier": "com.apple.CoreSimulator.SimDeviceType.iPad-A16", "productFamily": "iPad" });
        let device = |udid: &str, name: &str, kind: &str, available: bool| json!({ "udid": udid, "name": name, "state": state(udid), "isAvailable": available, "deviceTypeIdentifier": format!("com.apple.CoreSimulator.SimDeviceType.{kind}") });
        json!({
            "runtimes": [
                { "identifier": "ios-26-2", "platform": "iOS", "version": "26.2", "isAvailable": true, "supportedDeviceTypes": [iphone("iPhone-16e")] },
                { "identifier": "ios-26-10", "platform": "iOS", "version": "26.10", "isAvailable": true, "supportedDeviceTypes": [iphone("iPhone-17-Pro"), iphone("iPhone-17"), ipad] },
                { "identifier": "ios-27-0", "platform": "iOS", "version": "27.0", "isAvailable": false, "supportedDeviceTypes": [iphone("iPhone-18")] },
                { "identifier": "watchos-26-5", "platform": "watchOS", "version": "26.5", "isAvailable": true, "supportedDeviceTypes": [iphone("Watch")] },
            ],
            "devices": {
                "ios-26-2": [device("OLD", "iPhone 16e", "iPhone-16e", true)],
                "ios-26-10": [
                    device("IPAD", "iPad (A16)", "iPad-A16", true),
                    device("GONE", "iPhone 17 Pro", "iPhone-17-Pro", false),
                    device("NEW", "iPhone 17", "iPhone-17", true),
                    device("NEWER", "iPhone 17 Pro", "iPhone-17-Pro", true),
                ],
                "ios-27-0": [device("BETA", "iPhone 18", "iPhone-18", true)],
                "watchos-26-5": [device("WATCH", "Watch", "Watch", true)],
            },
        })
    }

    /// The simulator `where_to_run` takes, with the running one it passed over for when a run outside the
    /// lock was using one: `take`'s whole decision for a machine, read without a machine to look at.
    fn take_of(listing: &Value, users: &[&str]) -> (Device, Option<(Device, String)>) {
        let used = used_among(users);
        match where_to_run(&iphones(listing), &used).unwrap() {
            Choice::Take { device, instead_of } => (device, instead_of),
            Choice::Wait { device, because } => panic!("this test reads what is taken: {device:?} was waited for, {because}"),
        }
    }

    /// The look of a run outside the lock using the simulators `users` names, answering with the pid that
    /// names one — what the choosing tests need, since a look at processes and tests is all the real one
    /// does too.
    fn used_among<'a>(users: &'a [&'a str]) -> impl Fn(&Device) -> Result<Option<String>, String> + 'a {
        move |device| Ok(if users.contains(&device.udid.as_str()) { Some(used_by_processes(&["42".to_string()])) } else { None })
    }

    fn chosen(booted: &[&str]) -> String {
        take_of(&machine(booted), &[]).0.udid
    }

    #[test]
    fn a_running_iphone_is_taken_before_one_is_booted() {
        assert_eq!(chosen(&["OLD"]), "OLD", "a running iPhone, whatever its runtime, is taken rather than booting a second simulator");
        assert!(take_of(&machine(&["OLD"]), &[]).0.running);
        assert_eq!(chosen(&["OLD", "NEWER"]), "NEWER", "of two running, the newest runtime's");
        // Running, but no iPhone, or no device of this machine's Xcode: neither is taken.
        assert_eq!(chosen(&["IPAD", "WATCH", "GONE", "BETA"]), "NEW");
        let mut booting = machine(&[]);
        booting["devices"]["ios-26-2"][0]["state"] = json!("Booting");
        assert_eq!(take_of(&booting, &[]).0.udid, "OLD", "a simulator another run is booting is running already");
    }

    #[test]
    fn with_none_running_the_newest_runtimes_first_iphone_is_booted() {
        // 26.10 is newer than 26.2, which text would take for the newer; the runtime Xcode cannot run, the iPad
        // and the device whose runtime is gone are passed over, and `simctl`'s order stands within one.
        assert_eq!(take_of(&machine(&[]), &[]).0, Device { udid: "NEW".to_string(), name: "iPhone 17".to_string(), running: false });
    }

    #[test]
    fn a_machine_with_no_iphone_simulator_is_told_the_two_commands_that_make_one() {
        let mut bare = machine(&[]);
        bare["devices"] = json!({ "watchos-26-5": bare["devices"]["watchos-26-5"].clone(), "ios-26-10": [bare["devices"]["ios-26-10"][0].clone()] });
        let used = used_among(&[]);
        assert_eq!(
            where_to_run(&iphones(&bare), &used).unwrap_err(),
            "no iPhone simulator is available to run the Swift suite on: install the runtime with xcodebuild -downloadPlatform iOS and create a device with xcrun simctl create"
        );
        assert!(where_to_run(&iphones(&json!({})), &used).is_err(), "a listing that holds nothing holds no simulator");
    }

    /// A running simulator a run outside the lock is using is not taken: the next available iPhone of its
    /// own runtime is, to boot under this run's lock, and the saying carries which was taken, which was
    /// left, and why.
    #[test]
    fn a_running_simulator_used_from_outside_the_lock_gives_its_place_to_one_of_its_runtime() {
        let (device, instead_of) = take_of(&machine(&["NEW"]), &["NEW"]);
        assert_eq!(device.udid, "NEWER", "the same runtime's next iPhone, not one of another runtime");
        let (busy, because) = instead_of.expect("the passed-over simulator and the why");
        assert_eq!(busy.udid, "NEW");
        assert_eq!(because, "a run started without a lock names it in its arguments (pid 42)");
    }

    /// With no other iPhone of the used one's runtime free, the run waits for the run using it to end:
    /// an iPhone of another runtime is left shut, and where two runtimes run, the wait is for the one
    /// this run would have taken first.
    #[test]
    fn without_another_of_its_runtime_the_run_waits_for_the_run_using_it() {
        let used = used_among(&["OLD"]);
        match where_to_run(&iphones(&machine(&["OLD"])), &used).unwrap() {
            Choice::Wait { device, because } => {
                assert_eq!(device.udid, "OLD", "no other iPhone of 26.2 is free to boot; an iPhone of 26.10 is not taken while OLD may free");
                assert_eq!(because, "a run started without a lock names it in its arguments (pid 42)");
            }
            taken => panic!("{taken:?} boots an iPhone of another runtime over a wait"),
        }
        let used = used_among(&["NEW", "NEWER", "OLD"]);
        match where_to_run(&iphones(&machine(&["NEW", "OLD"])), &used).unwrap() {
            Choice::Wait { device, .. } => {
                assert_eq!(device.udid, "NEW", "both runtimes run and all three are used: the newest runtime's is the one this run would have taken first")
            }
            taken => panic!("{taken:?}"),
        }
    }

    /// A running iPhone nobody outside the lock is using is taken before anything is booted, even when
    /// an iPhone of its own runtime stands free and a running one of another runtime is the used one.
    #[test]
    fn a_running_iphone_left_free_is_taken_before_another_is_booted() {
        let (device, instead_of) = take_of(&machine(&["NEW", "OLD"]), &["NEW"]);
        assert_eq!(device.udid, "OLD", "running and free: taken, as it ever was");
        assert!(instead_of.is_none(), "OLD is taken itself: nothing was passed over for it");
    }

    /// Nothing runs, and a run outside the lock has already named the first stopped iPhone as its
    /// destination and will boot it: it is left to that run, and the next free iPhone is booted instead.
    #[test]
    fn a_stopped_simulator_already_named_from_outside_the_lock_is_left_to_the_run_that_named_it() {
        let (device, instead_of) = take_of(&machine(&[]), &["NEW"]);
        assert_eq!(device.udid, "NEWER", "NEW boots to the run that already named it; its runtime's next iPhone boots here");
        assert!(instead_of.is_none(), "nothing was running to pass over: NEW was never this run's to take");
    }

    /// Every simulator is used and none runs: nothing to take, nothing to boot over, so the run waits for
    /// the run using the one the newest runtime offers first.
    #[test]
    fn when_every_simulator_is_used_the_run_waits_for_the_run_using_the_newest_runtimes_first() {
        let used = used_among(&["NEW", "NEWER", "OLD"]);
        match where_to_run(&iphones(&machine(&[])), &used).unwrap() {
            Choice::Wait { device, because } => {
                assert_eq!(device.udid, "NEW");
                assert_eq!(because, "a run started without a lock names it in its arguments (pid 42)");
            }
            taken => panic!("{taken:?} takes a simulator a run outside the lock is using"),
        }
    }

    /// The one wording of what a run outside the lock is using a simulator by, at the taking and at the
    /// shutdown alike.
    #[test]
    fn what_uses_a_simulator_from_outside_the_lock_says_so_in_one_wording() {
        assert_eq!(used_by_processes(&["4242".to_string(), "4300".to_string()]), "a run started without a lock names it in its arguments (pid 4242, 4300)");
        assert_eq!(
            used_by_tests(&["com.apple.xpc.launchd.oneshot.0x10000001.xctest".to_string()]),
            "a test of another run runs in it (com.apple.xpc.launchd.oneshot.0x10000001.xctest)"
        );
    }

    /// The one definition reads both ways a run outside the lock shows: by the processes naming the
    /// simulator, a run started by id from its first moment, and by the tests in a running simulator, a run
    /// started by name once its tests run; and a look that could not answer stops the decision.
    #[test]
    fn a_run_outside_the_lock_shows_by_a_process_naming_the_simulator_or_by_a_test_in_it() {
        let pid = ["4242".to_string()];
        let none = || Ok(Vec::new());
        let xctest = || Ok(vec!["com.apple.xpc.launchd.oneshot.0x10000001.xctest".to_string()]);
        let unasked = || Err("the simulator's launchd was asked".to_string());
        let by_id = "a run started without a lock names it in its arguments (pid 4242)";
        assert_eq!(in_use(&pid, true, none).unwrap().as_deref(), Some(by_id), "by id, before any test of it runs");
        assert_eq!(in_use(&pid, false, unasked).unwrap().as_deref(), Some(by_id), "by id, the simulator not yet booted");
        assert_eq!(in_use(&pid, true, unasked).unwrap().as_deref(), Some(by_id), "named already: its launchd is not asked");
        assert_eq!(
            in_use(&[], true, xctest).unwrap().as_deref(),
            Some("a test of another run runs in it (com.apple.xpc.launchd.oneshot.0x10000001.xctest)"),
            "by name, once its tests run"
        );
        assert_eq!(in_use(&[], true, none).unwrap(), None);
        assert_eq!(in_use(&[], false, unasked).unwrap(), None, "a simulator that is not running runs no test, and is not asked");
        assert!(in_use(&[], true, || Err("launchctl list failed".to_string())).is_err(), "no answer is no \"unused\"");
    }

    /// What the run says when it takes another simulator in place of the used one, and when it waits for
    /// the run using one to end: which it did, which simulator, and why.
    #[test]
    fn a_run_says_which_it_did_and_why() {
        let free = Device { udid: "BBBB".to_string(), name: "iPhone 17e".to_string(), running: false };
        let busy = Device { udid: "AAAA".to_string(), name: "iPhone 17 Pro".to_string(), running: true };
        let because = used_by_processes(&["42".to_string()]);
        assert_eq!(
            said_taking_instead(&free, &busy, &because),
            "simulator: the suite takes iPhone 17e (BBBB) instead of iPhone 17 Pro (AAAA): a run started without a lock names it in its arguments (pid 42)"
        );
        assert_eq!(
            said_waiting(&busy, &because),
            "simulator: the suite waits for iPhone 17 Pro (AAAA): a run started without a lock names it in its arguments (pid 42); no other iPhone of its runtime is free to boot, so it chooses again once that run ends"
        );
    }

    #[test]
    fn a_running_test_runner_is_a_test_in_the_simulator_and_nothing_else_is() {
        let table = "PID\tStatus\tLabel\n\
            17439\t0\tcom.apple.xpc.launchd.oneshot.0x10000001.xctest\n\
            -\t0\tcom.apple.xpc.launchd.oneshot.0x10000002.xctest\n\
            17396\t0\tcom.apple.testmanagerd\n\
            17196\t0\tUIKitApplication:com.apple.mobilecal[1ffd][rb-legacy]\n\
            17500\t0\tcom.example.notxctest\n";
        assert_eq!(test_jobs(table), ["com.apple.xpc.launchd.oneshot.0x10000001.xctest"]);
        assert!(test_jobs("PID\tStatus\tLabel\n17396\t0\tcom.apple.testmanagerd\n").is_empty());
    }

    /// `pgrep` answers in its exit code: 0 found processes, whose pids it printed, 1 found none, anything
    /// else no answer at all — and no answer stops a shutdown, rather than passing for "nobody uses it".
    #[test]
    fn pgreps_exit_says_whether_a_process_names_the_simulator() {
        assert_eq!(named_pids(Some(0), "4242\n4300\n"), Some(vec!["4242".to_string(), "4300".to_string()]));
        assert_eq!(named_pids(Some(1), ""), Some(Vec::new()));
        assert_eq!(named_pids(Some(2), "pgrep: bad flag\n"), None, "a pgrep that could not run answered nothing");
        assert_eq!(named_pids(None, ""), None, "a pgrep a signal ended answered nothing");
    }

    /// A run may spell the simulator's id any way `simctl` takes it, which is any case: a process naming
    /// it lower-case boots and shuts the simulator down just the same, so the look must name it too, as it
    /// names one spelling it as `simctl` prints it, the only spelling `xcodebuild` takes — and no process
    /// naming another simulator.
    #[test]
    fn a_run_naming_the_id_in_another_case_names_the_simulator_too() {
        use std::os::unix::process::CommandExt;
        let udid = "FEEDFEED-FEED-4FED-8FED-0000000FEEDF";
        let naming = |id: &str| {
            Command::new("/bin/sleep")
                .arg0(format!("xcodebuild -destination platform=iOS Simulator,id={id}"))
                .arg("20")
                .spawn()
                .expect("the test can start its own naming process")
        };
        let mut namers =
            [naming(udid), naming(&udid.to_lowercase()), naming("FeedFeed-feed-4fed-8FED-0000000feedf"), naming("FEEDFEED-FEED-4FED-8FED-0000000FEED0")];
        let seen = named_processes(udid);
        for namer in &mut namers {
            let _ = namer.kill();
            let _ = namer.wait();
        }
        let seen = seen.unwrap();
        let [upper, lower, mixed, other] = namers.map(|namer| seen.contains(&namer.id().to_string()));
        assert!(upper, "as simctl prints it: {seen:?}");
        assert!(lower, "in lower case: {seen:?}");
        assert!(mixed, "in mixed case: {seen:?}");
        assert!(!other, "another simulator's id is no use of this one: {seen:?}");
    }

    /// Under its lock a run passes over a simulator that is gone, one a run outside the lock uses — asked
    /// with the state the listing under the lock reads — and one somebody else booted as this run booted
    /// it; it takes a running one with the boot it had, and boots a stopped one itself.
    #[test]
    fn under_its_lock_a_run_passes_over_what_a_run_outside_the_lock_uses_or_booted_under_it() {
        use std::cell::RefCell;
        let mut listed = machine(&["NEW"]);
        listed["devices"]["ios-26-10"][2]["lastBootedAt"] = json!("T1");
        let device = |udid: &str| Device { udid: udid.to_string(), name: "iPhone 17".to_string(), running: false };
        let asked = RefCell::new(Vec::new());
        let booted = RefCell::new(Vec::new());
        let held = |udid: &str, using: bool, boots: bool| {
            let used = |device: &Device, running: bool| {
                asked.borrow_mut().push(format!("{} {running}", device.udid));
                Ok(using.then(|| used_by_processes(&["42".to_string()])))
            };
            let boot = |device: &Device| {
                booted.borrow_mut().push(device.udid.clone());
                Ok(boots)
            };
            under_the_lock(&listed, &device(udid), &used, &boot).unwrap()
        };
        assert_eq!(held("NEW", false, true), Held::Taken(Some("T1".to_string())), "running and free: taken with the boot it had");
        assert_eq!(held("NEW", true, true), Held::PassedOver, "running and used from outside the lock");
        assert_eq!(held("NEWER", true, true), Held::PassedOver, "stopped, and named by a run outside the lock");
        assert_eq!(held("NEWER", false, true), Held::Taken(None), "stopped and free: booted by this run");
        assert_eq!(held("NEWER", false, false), Held::PassedOver, "booted by somebody else as this run booted it");
        assert_eq!(held("NOSUCH", false, true), Held::PassedOver, "gone while the run waited for it");
        assert_eq!(*asked.borrow(), ["NEW true", "NEW true", "NEWER false", "NEWER false", "NEWER false"], "asked with the state read under the lock");
        assert_eq!(*booted.borrow(), ["NEWER", "NEWER"], "only a stopped simulator no run outside the lock names is booted");
        let failing = |_: &Device, _: bool| Err("pgrep could not say".to_string());
        assert!(under_the_lock(&listed, &device("NEW"), &failing, &|_| Ok(true)).is_err(), "a look that could not answer takes nothing");
    }

    /// The wait looks at the simulator again after each pause until the look clears it, and a look that
    /// could not answer ends it with its failure.
    #[test]
    fn a_wait_looks_again_after_each_pause_until_the_simulator_is_free() {
        let wait = |answers: Vec<Result<Option<String>, String>>| {
            let mut answers = answers.into_iter();
            let (mut looks, mut pauses) = (0, 0);
            let mut look = || {
                looks += 1;
                answers.next().unwrap_or(Ok(None))
            };
            let ended = until_unused(&mut look, &mut || pauses += 1);
            (ended, looks, pauses)
        };
        let used = || Ok(Some(used_by_tests(&["com.apple.xpc.launchd.oneshot.0x10000001.xctest".to_string()])));
        assert_eq!(wait(vec![used(), used(), Ok(None)]), (Ok(()), 3, 2));
        assert_eq!(wait(vec![Ok(None)]), (Ok(()), 1, 0), "free at once: no pause");
        assert_eq!(wait(vec![used(), Err("pgrep could not say".to_string())]), (Err("pgrep could not say".to_string()), 2, 1));
    }

    /// `launchd_sim` is the device's own job, started when the device boots, and not a run that took the
    /// simulator: a list of processes naming it means nothing once it is the only one on it.
    #[test]
    fn the_simulators_own_launchd_is_not_a_run_naming_it() {
        assert_eq!(without_launchd(&["4242".to_string(), "4300".to_string()], &["4300".to_string()]), ["4242".to_string()]);
        assert!(
            without_launchd(&["4300".to_string()], &["4300".to_string()]).is_empty(),
            "only the device's own launchd names it: the device is up, but nothing uses it"
        );
        assert_eq!(without_launchd(&["4242".to_string()], &[]), ["4242".to_string()]);
    }

    /// What one run's lock file says while it holds the simulator, at each boot the simulator may have.
    #[test]
    fn every_boot_under_a_runs_lock_is_the_runs_but_the_one_somebody_else_had_made() {
        // Found stopped, so booted by the run: the run's, and still the run's at another boot when it ends,
        // since it went down and its own `bootstatus -b` or program booted it again.
        assert_eq!(theirs(None, "booted at T0\n"), None);
        assert_eq!(said_while_running(None, "T1"), "booted at T1\n");
        assert_eq!(said_while_running(None, "T2"), "booted at T2\n", "booted again while the run held it");
        // Found running with a boot no run caused, a hand-started run's, whatever the file said before: theirs.
        for said in ["", "booted at T0\n", "garbled", "booted at T1"] {
            assert_eq!(theirs(Some("T1"), said), Some("T1"), "{said:?}");
        }
        assert_eq!(said_while_running(Some("T1"), "T1"), "", "a hand-started run's boot is left to it");
        // That boot went down and the simulator was booted again while the run held it: the run's.
        assert_eq!(said_while_running(Some("T1"), "T2"), "booted at T2\n", "a boot made under the run's lock, after the one it found");
        // Found running with the boot a run left running for another's use: the runs'.
        assert_eq!(theirs(Some("T1"), "booted at T1\n"), None);
    }

    /// The lock file across runs, each run taking the simulator as the last one left it.
    #[test]
    fn a_boot_left_running_passes_to_the_next_run_and_one_somebody_else_made_never_does() {
        let next = |running: &str, said: &str| said_while_running(theirs(Some(running), said), running);
        // A run boots it, at T1, and leaves it running for a hand-started run that names it: the next run
        // finds that boot, takes it as the runs', and shuts it down once nothing uses it.
        let left = said_while_running(None, "T1");
        assert_eq!(next("T1", &left), "booted at T1\n");
        // After that shutdown somebody else boots it, at T2: no run's, then or in any run after.
        let found = next("T2", &left);
        assert_eq!(found, "");
        assert_eq!(next("T2", &found), "");
        // Two runs back to back: the second finds it stopped, whatever the first left in the file.
        assert_eq!(said_while_running(theirs(None, &left), "T3"), "booted at T3\n");
    }

    #[test]
    fn a_simulator_has_its_boot_while_it_is_booted_or_booting() {
        let mut listing = machine(&["OLD"]);
        listing["devices"]["ios-26-2"][0]["lastBootedAt"] = json!("2026-11-18T14:19:05Z");
        assert_eq!(booted_at(&listing, "OLD").as_deref(), Some("2026-11-18T14:19:05Z"));
        assert_eq!(booted_at(&listing, "NEW"), None, "shut down, it has no boot to compare");
        listing["devices"]["ios-26-10"][2]["state"] = json!("Booting");
        assert_eq!(booted_at(&listing, "NEW").as_deref(), Some(""), "booting, before simctl records the time");
        assert_eq!(booted_at(&listing, "NOSUCH"), None);
        // A record holds for the one boot it names: another boot a later run finds running is not a run's.
        assert_ne!(booted_by_a_run("2026-11-18T14:19:05Z"), booted_by_a_run("2026-11-18T15:02:11Z"));
    }

    /// A program that names the simulator nowhere is refused, before any simulator is chosen or booted.
    #[test]
    fn a_program_that_names_no_simulator_is_refused() {
        let refusal = "names no program to run with {simulator} where the simulator's id goes: cargo xtask simulator -- <program> <argument>…";
        assert_eq!(naming_the_simulator(&[]).unwrap_err(), refusal);
        assert_eq!(naming_the_simulator(&["xcodebuild", "-destination", "platform=iOS Simulator", "test-without-building"]).unwrap_err(), refusal);
        assert_eq!(naming_the_simulator(&["{simulator}"]).unwrap_err(), refusal, "the program itself is no argument that names it");
        let named = ["xcodebuild", "-destination", "platform=iOS Simulator,id={simulator}"];
        assert_eq!(naming_the_simulator(&named).unwrap(), ("xcodebuild", &named[1..]));
    }

    #[test]
    fn the_id_goes_where_each_argument_says() {
        assert_eq!(
            with_simulator(&["-destination", "platform=iOS Simulator,id={simulator}", "test-without-building"], "E334"),
            ["-destination", "platform=iOS Simulator,id=E334", "test-without-building"]
        );
    }
}
