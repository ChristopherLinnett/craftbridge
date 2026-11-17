//! The workspace's bindgen, run in library mode on the distribution library an app links, writes the
//! core's Swift and Kotlin with every exported type and call, under names both languages can hold.
//!
//! No gate stage compiles generated bindings until the Swift package and the Android build take them
//! in, so this reads what the generator wrote for the declarations it must hold, line by line as
//! UniFFI writes them, with runs of spaces read as one.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The distribution library cargo built for this test. A dev-dependency's library is built into the
/// folder the test's own executable is built in, under the name the platform gives a dynamic library.
fn distribution_library() -> Result<PathBuf, String> {
    let test = std::env::current_exe().map_err(|error| format!("the test cannot find its own path: {error}"))?;
    let folder = test.parent().ok_or_else(|| format!("{} sits in no folder", test.display()))?;
    let library = folder.join(format!("{}craftbridge_effects_ffi{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX));
    if library.is_file() { Ok(library) } else { Err(format!("no distribution library at {}", library.display())) }
}

/// Run the workspace's bindgen for `language` into `out`, as the generators of `cargo xtask generate`
/// run it.
fn generate(language: &str, out: &Path) -> Result<(), String> {
    let ran = Command::new(env!("CARGO_BIN_EXE_uniffi-bindgen"))
        .args(["generate", "--no-format", "--library"])
        .arg(distribution_library()?)
        .args(["--language", language, "--out-dir"])
        .arg(out)
        .output()
        .map_err(|error| format!("the bindgen does not start: {error}"))?;
    if ran.status.success() { Ok(()) } else { Err(format!("the bindgen failed for {language}: {}", String::from_utf8_lossy(&ran.stderr))) }
}

/// The file's lines, trimmed, with each run of spaces read as one.
fn lines(file: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(file).map_err(|error| format!("{} was not written: {error}", file.display()))?;
    Ok(text.lines().map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ")).collect())
}

/// The lines of `expected` that `file` lacks.
fn missing<'a>(file: &[String], expected: &[&'a str]) -> Vec<&'a str> {
    expected.iter().copied().filter(|line| !file.iter().any(|have| have == line)).collect()
}

/// The text of each `CraftError` variant that carries one, in the order UniFFI writes them.
const TEXT_VARIANTS: [&str; 7] = ["InvalidParams", "CommandFailed", "NotFound", "Unsupported", "Io", "Gpu", "Internal"];

#[test]
fn the_core_s_kotlin_holds_every_export_and_no_name_kotlin_owns() {
    let out = tempfile::tempdir().expect("a scratch folder");
    generate("kotlin", out.path()).unwrap();
    // The package path comes from the core's uniffi.toml, and the file at all from its scaffolding
    // being linked into the distribution library: without it library mode finds no core to write.
    let kotlin = lines(&out.path().join("io/github/christopherlinnett/craftbridge/ffi/core/craftbridge_core_ffi.kt")).unwrap();
    let expected = [
        "package io.github.christopherlinnett.craftbridge.ffi.core",
        // The one native library, named by the library the bindings are generated from.
        "return \"craftbridge_effects_ffi\"",
        "public interface EngineInterface {",
        "suspend fun `request`(`method`: kotlin.String, `params`: kotlin.String): kotlin.String",
        "suspend fun `requestWithProgress`(`method`: kotlin.String, `params`: kotlin.String, `listener`: ProgressListener, `cancel`: CancelToken): kotlin.String",
        "suspend fun `frame`(`params`: kotlin.String, `format`: PixelFormat): FrameBytes",
        "suspend fun `mcp`(`line`: kotlin.String): kotlin.String?",
        "suspend fun `suspendGpu`()",
        "suspend fun `resumeGpu`(): kotlin.String?",
        "suspend fun `shutDown`()",
        "fun `recovery`(): Recovery",
        "public interface CancelTokenInterface {",
        "fun `cancel`()",
        "fun `isCancelled`(): kotlin.Boolean",
        "public interface ProgressListener {",
        "fun `progress`(`done`: kotlin.ULong, `total`: kotlin.ULong, `secondsLeft`: kotlin.Double?)",
        "data class Folders (",
        "data class FrameBytes (",
        "data class Recovery (",
        "var `removed`: List<kotlin.String>",
        "var `refused`: List<RecoveryNote>",
        "data class RecoveryNote (",
        "var `reason`: kotlin.String",
        "var `data`: kotlin.ByteArray",
        "enum class GpuChoice {",
        "enum class PixelFormat {",
        "sealed class CraftException: kotlin.Exception() {",
    ];
    assert_eq!(missing(&kotlin, &expected), Vec::<&str>::new(), "the Kotlin bindings lack these");
    // Every generated object is an `AutoCloseable` whose own `close()` frees its handle, and every
    // generated exception overrides `kotlin.Exception.message`: an exported method `close`, or a
    // variant field `message`, collides with them and the bindings do not compile. UniFFI writes every
    // exported name in backticks and its own members without, so these lines are only ever exports.
    for owned in ["fun `close`(", "val `message`:"] {
        assert!(!kotlin.iter().any(|line| line.contains(owned)), "an export takes a name Kotlin's own `{owned}` holds");
    }
    let details = kotlin.iter().filter(|line| line.as_str() == "val `detail`: kotlin.String" || line.as_str() == "val `detail`: kotlin.String,").count();
    assert_eq!(details, TEXT_VARIANTS.len(), "every variant with text carries it as `detail`");
}

#[test]
fn the_core_s_swift_holds_every_export_in_files_named_for_its_crate() {
    let out = tempfile::tempdir().expect("a scratch folder");
    generate("swift", out.path()).unwrap();
    // Named for the crate, so the distribution crate's own Swift, generated into the same folder, never
    // overwrites them.
    for file in ["craftbridge_core_ffiFFI.h", "craftbridge_core_ffiFFI.modulemap"] {
        assert!(out.path().join(file).is_file(), "{file} was not written");
    }
    let swift = lines(&out.path().join("craftbridge_core_ffi.swift")).unwrap();
    let mut expected = vec![
        "public protocol EngineProtocol: AnyObject, Sendable {",
        "func request(method: String, params: String) async throws -> String",
        "func requestWithProgress(method: String, params: String, listener: ProgressListener, cancel: CancelToken) async throws -> String",
        "func frame(params: String, format: PixelFormat) async throws -> FrameBytes",
        "func mcp(line: String) async throws -> String?",
        "func suspendGpu() async throws",
        "func resumeGpu() async throws -> String?",
        "func shutDown() async throws",
        "func recovery() throws -> Recovery",
        "public protocol CancelTokenProtocol: AnyObject, Sendable {",
        "func cancel()",
        "func isCancelled() -> Bool",
        "public protocol ProgressListener: AnyObject, Sendable {",
        "func progress(done: UInt64, total: UInt64, secondsLeft: Double?)",
        "public struct Folders: Equatable, Hashable {",
        "public struct FrameBytes: Equatable, Hashable {",
        "public struct Recovery: Equatable, Hashable {",
        "public var removed: [String]",
        "public var refused: [RecoveryNote]",
        "public struct RecoveryNote: Equatable, Hashable {",
        "public var reason: String",
        "public var data: Data",
        "public enum GpuChoice: Equatable, Hashable {",
        "public enum PixelFormat: Equatable, Hashable {",
        // UniFFI writes the error's `public` on a line of its own.
        "enum CraftError: Swift.Error, Equatable, Hashable, Foundation.LocalizedError {",
        "case CommandFailed(command: String, detail: String, step: UInt32?",
    ];
    let single: Vec<String> =
        TEXT_VARIANTS.iter().filter(|variant| **variant != "CommandFailed").map(|variant| format!("case {variant}(detail: String")).collect();
    expected.extend(single.iter().map(String::as_str));
    assert_eq!(missing(&swift, &expected), Vec::<&str>::new(), "the Swift bindings lack these");
}
