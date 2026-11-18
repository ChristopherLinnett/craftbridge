//! The iPhone simulator the Swift Testing suite runs on: `cargo xtask simulator -- <program> <argument>…`
//! runs the program with every `{simulator}` in its arguments replaced by the id of one of the machine's
//! own iPhone simulators, and exits as the program does.
//!
//! The simulators are the machine's, shared by every run on it — another worktree's gate, a developer's or
//! an agent's own test run — so nothing here creates, erases or renames one. A run takes one that is
//! running already when there is one, and boots one only when none is: the newest iOS runtime's, so the
//! choice never rests on the order `simctl` happens to list its runtimes in. It shuts down only a
//! simulator whose boot a run of this command caused, itself or through its program, and only once nothing uses it: no run of this command holds
//! it, no run started without a lock names it in its arguments, and no package's tests run in it,
//! whoever started them.
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
//! over.
//! While its tests run they show in the simulator: a Swift package's tests run in Xcode's test runner,
//! `xctest`, a job of the device's own launchd. And from the moment it starts it shows in the machine's
//! processes, because such a run carries the simulator's id in its own arguments — what
//! `xcodebuild -destination id=` and `simctl` are started with — which a run about to shut the simulator
//! down asks `pgrep` before it does, saving the device's own `launchd_sim`, which names it merely by the
//! device being up. What stays invisible is a run naming its destination by device
//! name, not id, before its tests start: the name belongs to no one simulator, so no rule can tell that
//! such a run would take this one — and no harm is done if the simulator is shut down under it, since
//! `xcodebuild` boots its destination again for its tests.
//!
//! What the machine holds is read from `simctl`'s own JSON (`xcrun simctl list --json`): which runtimes are
//! iOS's and available, which device types are iPhones (the runtime's own `productFamily`), and each
//! device's state. `launchctl list` has no other form than its table of jobs, a pid or `-`, a status and a
//! label, which is what is read of it.

use serde_json::Value;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// What a program's arguments say where the simulator's id goes.
const PLACEHOLDER: &str = "{simulator}";

/// The variable naming the folder of the locks, which `tools/env.sh` sets for every run on the machine.
const LOCKS: &str = "CRAFTBRIDGE_SIMULATOR_LOCKS";

/// The tool that answers whether any process on the machine matches a pattern in its arguments.
const PGREP: &str = "pgrep";

/// The launchd daemon a booted simulator runs its jobs through, named by no run: it carries its device's
/// id in its own arguments for as long as the device is booted, so it says the device is up, not used.
const LAUNCHD: &str = "launchd_sim";

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
/// it (or that beat the run's own `simctl boot`), unless a run of this command caused that boot. A run
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
#[derive(Debug, PartialEq)]
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
    let locks = locks_folder()?;
    std::fs::create_dir_all(&locks).map_err(|error| format!("could not create {}: {error}", locks.display()))?;
    let (device, theirs, using) = take(&locks)?;
    let marker = lock_file(&locks, &device);
    let ran = ready(&marker, &device, theirs.as_deref()).and_then(|()| {
        println!("simulator: the suite runs on {} ({})", device.name, device.udid);
        Command::new(program).args(with_simulator(arguments, &device.udid)).status().map_err(|error| format!("could not run {program}: {error}"))
    });
    let left = leave(&marker, &device, theirs.as_deref(), using);
    let status = ran?;
    left?;
    Ok(crate::report::exit_of("simulator", program, status))
}

/// The simulator to run on, held whole so that this run is alone on it: the listing says which `choose`
/// chooses, and the simulator's lock — which waits out any run whose program is still testing — says
/// when it is this run's turn. Another run may have booted, tested on and shut down a listed running
/// simulator while this one waited, so under the lock the state is looked at again: a simulator found
/// stopped is booted here, which no other run of this command can be doing at the same time because it
/// is this run that holds the simulator's lock — so the boot is this run's to record, and to undo. Taken
/// with it is the boot somebody else had made, running when this run looked, which stays theirs.
fn take(locks: &Path) -> Result<(Device, Option<String>, File), String> {
    loop {
        let device = choose(&listing()?)?;
        let marker = lock_file(locks, &device);
        if busy_with_it(&marker)? {
            println!("simulator: the suite waits for {} ({}): another run is testing on it", device.name, device.udid);
        }
        let using = crate::filelock::hold(&marker)?;
        let listed = listing()?;
        if simulator(&listed, &device.udid).is_none() {
            continue; // the simulator is gone, its runtime deleted while this run waited for it
        }
        let running = match booted_at(&listed, &device.udid) {
            Some(at) => Some(at),
            None => boot(&device)?,
        };
        let said = std::fs::read_to_string(&marker).unwrap_or_default();
        return Ok((device, theirs(running.as_deref(), &said).map(str::to_string), using));
    }
}

/// The simulator's lock file, in the folder of the locks.
fn lock_file(locks: &Path, device: &Device) -> PathBuf {
    locks.join(format!("{}.lock", device.udid))
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

/// Whether another run is testing on this simulator: its lock file exists and someone holds it. A run
/// found busy is said for before this one waits behind it, so the queue shows in the stage's log.
fn busy_with_it(marker: &Path) -> Result<bool, String> {
    let Ok(held) = File::open(marker) else { return Ok(false) };
    crate::filelock::try_lock(&held).map_err(|error| format!("could not lock {}: {error}", marker.display())).map(|free| !free)
}

/// The folder `tools/env.sh` names for the locks.
fn locks_folder() -> Result<PathBuf, String> {
    std::env::var_os(LOCKS)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("{LOCKS} is not set to an absolute path: run through tools/env.sh, which names the folder where the runs on this machine take turns on its simulators"))
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

/// The simulator to run on: a running iPhone if there is one, so a run never boots a second simulator
/// beside one already running, the newest runtime's first; otherwise the first iPhone of the newest
/// available iOS runtime. Only iPhones, as their runtime names its device types: the suite is the iPhone's
/// (plan §28). Only what `simctl` marks available, which leaves out a device whose runtime is gone, or
/// one this Xcode cannot run.
fn choose(listing: &Value) -> Result<Device, String> {
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
    let taken = found.iter().position(|(_, device)| device.running).unwrap_or(0);
    found.into_iter().nth(taken).map(|(_, device)| device).ok_or_else(|| {
        "no iPhone simulator is available to run the Swift suite on: install the runtime with xcodebuild -downloadPlatform iOS and create a device with xcrun simctl create"
            .to_string()
    })
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

/// Boot the simulator: `None` when this run booted it, and the boot `simctl` records when another run
/// booted it first, which is no failure, since the simulator is then running as this run needs it. Only a
/// run that takes no lock can do that, an `xcodebuild` booting its destination, so that boot is that run's.
fn boot(device: &Device) -> Result<Option<String>, String> {
    let output = Command::new("xcrun").args(["simctl", "boot", &device.udid]).output().map_err(|error| format!("could not run xcrun simctl boot: {error}"))?;
    if output.status.success() {
        return Ok(None);
    }
    if let Some(at) = booted_at(&listing()?, &device.udid) {
        return Ok(Some(at));
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
    let users = named_processes(&device.udid)?;
    if !users.is_empty() {
        println!("simulator: {} is left running: a run started without a lock names it in its arguments (pid {})", device.name, users.join(", "));
        return Ok(());
    }
    let tests = tests_running(device)?;
    if !tests.is_empty() {
        println!("simulator: {} is left running: a test of another run runs in it ({})", device.name, tests.join(", "));
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

/// The tests running in the simulator, by their launchd jobs.
fn tests_running(device: &Device) -> Result<Vec<String>, String> {
    let output = Command::new("xcrun")
        .args(["simctl", "spawn", &device.udid, "launchctl", "list"])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not ask the iPhone simulator {} ({}) what it runs: {error}", device.name, device.udid))?;
    if !output.status.success() {
        return Err(format!("could not ask the iPhone simulator {} ({}) what it runs: launchctl list failed ({})", device.name, device.udid, output.status));
    }
    Ok(test_jobs(&String::from_utf8_lossy(&output.stdout)))
}

/// Which processes of this machine name the simulator in their arguments: a run started without a lock —
/// an `xcodebuild -destination id=` or a `simctl` aimed at the device, started by hand — carries its id
/// from the moment it starts, while inside the device it shows only once its tests run. A decision to
/// shut down waits until none does, save the device's own launchd daemon, which names it merely by being
/// booted.
fn named_processes(udid: &str) -> Result<Vec<String>, String> {
    let found = pgrep(&["-f", udid]).ok_or_else(|| format!("{PGREP} could not say which processes name the simulator {udid}"))?;
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

    fn chosen(booted: &[&str]) -> String {
        choose(&machine(booted)).unwrap().udid
    }

    #[test]
    fn a_running_iphone_is_taken_before_one_is_booted() {
        assert_eq!(chosen(&["OLD"]), "OLD", "a running iPhone, whatever its runtime, is taken rather than booting a second simulator");
        assert!(choose(&machine(&["OLD"])).unwrap().running);
        assert_eq!(chosen(&["OLD", "NEWER"]), "NEWER", "of two running, the newest runtime's");
        // Running, but no iPhone, or no device of this machine's Xcode: neither is taken.
        assert_eq!(chosen(&["IPAD", "WATCH", "GONE", "BETA"]), "NEW");
        let mut booting = machine(&[]);
        booting["devices"]["ios-26-2"][0]["state"] = json!("Booting");
        assert_eq!(choose(&booting).unwrap().udid, "OLD", "a simulator another run is booting is running already");
    }

    #[test]
    fn with_none_running_the_newest_runtimes_first_iphone_is_booted() {
        // 26.10 is newer than 26.2, which text would take for the newer; the runtime Xcode cannot run, the iPad
        // and the device whose runtime is gone are passed over, and `simctl`'s order stands within one.
        assert_eq!(choose(&machine(&[])).unwrap(), Device { udid: "NEW".to_string(), name: "iPhone 17".to_string(), running: false });
    }

    #[test]
    fn a_machine_with_no_iphone_simulator_is_told_the_two_commands_that_make_one() {
        let mut bare = machine(&[]);
        bare["devices"] = json!({ "watchos-26-5": bare["devices"]["watchos-26-5"].clone(), "ios-26-10": [bare["devices"]["ios-26-10"][0].clone()] });
        assert_eq!(
            choose(&bare).unwrap_err(),
            "no iPhone simulator is available to run the Swift suite on: install the runtime with xcodebuild -downloadPlatform iOS and create a device with xcrun simctl create"
        );
        assert!(choose(&json!({})).is_err(), "a listing that holds nothing holds no simulator");
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
