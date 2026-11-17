//! D-8, pinned as Cargo's own answer: boa's `jsvalue-enum` feature — the layout that tolerates
//! Android's top-byte-tagged heap pointers (plan/evidence/03-device-feasibility.md Finding 9) — is
//! enabled through this crate's target-specific dependency, so the Android build resolves boa with
//! that feature added and nothing else, and no other target the workspace builds changes boa at
//! all. `cargo tree --target` is the ground truth: it answers, for one target, exactly the features
//! that build turns on for a package. The targets are the host and every one the toolchain pin
//! lists, and which of them is Android is the compiler's own answer for the target. Helpers of a
//! test answer `Result` and leave the unwrapping to the `#[test]` functions, as everywhere else.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// What a command printed, or why it did not run to success.
fn printed(command: &mut Command, what: &str) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{what} runs: {e}"))?;
    if !out.status.success() {
        return Err(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The features boa_engine builds with for one target (`None`: the host), as Cargo resolves that
/// build's graph from the committed lock, which the run may not rewrite.
fn boa_features(target: Option<&str>) -> Result<BTreeSet<String>, String> {
    let mut tree = Command::new(env!("CARGO"));
    tree.args(["tree", "--locked", "-p", "craftbridge-effects", "-i", "boa_engine", "-e", "normal", "--depth", "0", "--prefix", "none", "--format", "{f}"]);
    if let Some(target) = target {
        tree.args(["--target", target]);
    }
    let features = printed(&mut tree, &format!("`cargo tree` for {target:?}"))?;
    Ok(features.trim().split(',').filter(|feature| !feature.is_empty()).map(str::to_string).collect())
}

/// The features boa builds with on the host, which is no Android build, so it never carries the
/// Android-only feature: what every other target is held to.
fn host_features() -> Result<BTreeSet<String>, String> {
    let host = boa_features(None)?;
    if host.contains("jsvalue-enum") {
        return Err(format!("the host must not carry the Android-only feature (D-8): {host:?}"));
    }
    Ok(host)
}

/// Each target `rust/rust-toolchain.toml` pins for the phones, read with the TOML parser, with
/// whether the compiler builds it for Android.
fn pinned_targets() -> Result<Vec<(String, bool)>, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pin: toml::Table = std::fs::read_to_string(root.join("rust-toolchain.toml"))
        .map_err(|e| format!("the toolchain pin reads: {e}"))?
        .parse()
        .map_err(|e| format!("the toolchain pin parses: {e}"))?;
    let listed =
        pin.get("toolchain").and_then(|toolchain| toolchain.get("targets")).and_then(toml::Value::as_array).ok_or("the toolchain pin lists its targets")?;
    // The compiler of the toolchain this test runs under, beside the cargo that runs it.
    let rustc = Path::new(env!("CARGO")).with_file_name("rustc");
    listed
        .iter()
        .map(|target| {
            let target = target.as_str().ok_or("each pinned target is a string")?;
            let cfg = printed(Command::new(&rustc).args(["--print", "cfg", "--target", target]), &format!("`rustc --print cfg --target {target}`"))?;
            Ok((target.to_string(), cfg.lines().any(|line| line == "target_os=\"android\"")))
        })
        .collect()
}

/// Android takes the feature, and only the feature: the device-feasibility finding is an Android
/// panic, which the feature removes, and an Android boa with any feature the desktop's lacks would
/// run scripts and expressions the command line, the conformance oracle, cannot.
#[test]
fn android_builds_boa_with_jsvalue_enum_and_nothing_more() {
    let mut expected = host_features().expect("the host graph answers");
    expected.insert("jsvalue-enum".to_string());
    let android: Vec<String> =
        pinned_targets().expect("the pinned targets answer").into_iter().filter_map(|(target, android)| android.then_some(target)).collect();
    assert!(!android.is_empty(), "the toolchain pins an Android target, or this test proves nothing");
    for target in android {
        assert_eq!(
            boa_features(Some(&target)).expect("the Android graph answers"),
            expected,
            "`{target}` must build boa as the host does, plus `jsvalue-enum` (D-8)"
        );
    }
}

/// No other target asks for anything: iOS and the simulator build boa exactly as the host does,
/// because this crate requests the feature only under `cfg(target_os = "android")`.
#[test]
fn no_other_target_changes_boa() {
    let host = host_features().expect("the host graph answers");
    let others: Vec<String> =
        pinned_targets().expect("the pinned targets answer").into_iter().filter_map(|(target, android)| (!android).then_some(target)).collect();
    assert!(!others.is_empty(), "the toolchain pins a target that is not Android, or this test proves nothing");
    for target in others {
        assert_eq!(boa_features(Some(&target)).expect("the graph answers"), host, "`{target}` must build boa as the host does (D-8)");
    }
}
