//! `cargo xtask elf-alignment <llvm-readelf> <library>…`: every native library an Android build ships is
//! aligned to the 16 KB pages Android 15 maps libraries with (plan V6).
//!
//! The NDK's own `llvm-readelf` reads the program headers, as JSON, and this judges what it printed: every
//! `PT_LOAD` segment of a library must have an alignment that is a multiple of 16384, since a device with
//! 16 KB pages cannot map a segment aligned to 4 KB. The NDK aligns to that by default from r28
//! (`tools/env.sh` pins the major), so a library that is not was linked by another toolchain or told to.

use serde_json::Value;
use std::path::Path;
use std::process::{Command, ExitCode};

/// Android's page size on the devices that map libraries at 16 KB.
const PAGE: u64 = 16384;

/// Judge one library's program headers, as `llvm-readelf --program-headers --elf-output-style=JSON` prints
/// them: how many loadable segments it has, or the first that is not aligned to [`PAGE`].
fn judge(readelf_json: &str) -> Result<usize, String> {
    let report: Value = serde_json::from_str(readelf_json).map_err(|error| format!("llvm-readelf printed no JSON: {error}"))?;
    let headers = report
        .as_array()
        .and_then(|files| files.first())
        .and_then(|file| file.get("ProgramHeaders"))
        .and_then(Value::as_array)
        .ok_or_else(|| "llvm-readelf printed no program headers".to_string())?;
    let mut loads = 0;
    for header in headers.iter().filter_map(|entry| entry.get("ProgramHeader")) {
        if header.get("Type").and_then(|kind| kind.get("Name")).and_then(Value::as_str) != Some("PT_LOAD") {
            continue;
        }
        loads += 1;
        let alignment = header.get("Alignment").and_then(Value::as_u64).ok_or_else(|| format!("PT_LOAD segment {loads} has no alignment"))?;
        if alignment == 0 || alignment % PAGE != 0 {
            return Err(format!("PT_LOAD segment {loads} is aligned to {alignment} bytes, not to a multiple of the {PAGE} bytes of Android's 16 KB pages"));
        }
    }
    if loads == 0 {
        return Err("it holds no PT_LOAD segment, so it is no library".to_string());
    }
    Ok(loads)
}

/// Ask `readelf` for `library`'s program headers and judge them.
fn check(readelf: &Path, library: &Path) -> Result<usize, String> {
    let output = Command::new(readelf)
        .args(["--program-headers", "--elf-output-style=JSON"])
        .arg(library)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", readelf.display()))?;
    if !output.status.success() {
        return Err(format!("{} exited with {}: {}", readelf.display(), output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    judge(&String::from_utf8_lossy(&output.stdout))
}

/// `cargo xtask elf-alignment`: every library named is checked even after one fails, so one run names each
/// library that is not aligned. No library named is a failure: a check that read nothing must not pass.
pub(crate) fn run(readelf: &str, libraries: &[&str]) -> ExitCode {
    if libraries.is_empty() {
        println!("elf-alignment: no library to check");
        return ExitCode::from(1);
    }
    let mut failed = false;
    for library in libraries {
        match check(Path::new(readelf), Path::new(library)) {
            Ok(loads) => println!("elf-alignment: {library}: {loads} loadable segments, each aligned to a multiple of {PAGE} bytes"),
            Err(reason) => {
                failed = true;
                println!("elf-alignment: {library}: {reason}");
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `llvm-readelf` prints for a library with these `PT_LOAD` alignments, and a note segment
    /// whose own alignment is small, as every library has.
    fn headers(alignments: &[u64]) -> String {
        let loads: Vec<String> =
            alignments.iter().map(|alignment| format!(r#"{{"ProgramHeader":{{"Type":{{"Name":"PT_LOAD","Value":1}},"Alignment":{alignment}}}}}"#)).collect();
        let note = r#"{"ProgramHeader":{"Type":{"Name":"PT_NOTE","Value":4},"Alignment":4}}"#;
        let all: Vec<&str> = std::iter::once(note).chain(loads.iter().map(String::as_str)).collect();
        format!(r#"[{{"FileSummary":{{"File":"lib.so"}},"ProgramHeaders":[{}]}}]"#, all.join(","))
    }

    #[test]
    fn a_library_aligned_to_the_16_kb_pages_passes_whatever_its_other_segments_hold() {
        assert_eq!(judge(&headers(&[16384, 16384, 16384])), Ok(3));
        assert_eq!(judge(&headers(&[65536])), Ok(1), "a larger multiple maps at 16 KB too");
    }

    #[test]
    fn a_segment_aligned_to_less_is_named_by_its_place() {
        assert!(judge(&headers(&[16384, 4096])).is_err_and(|error| error.starts_with("PT_LOAD segment 2 is aligned to 4096 bytes")));
        assert!(judge(&headers(&[0])).is_err(), "no alignment is no 16 KB alignment");
        assert!(judge(&headers(&[20000])).is_err(), "a multiple of the page is what maps, not any larger number");
    }

    #[test]
    fn output_that_is_not_a_library_fails_instead_of_passing_unread() {
        assert!(judge(&headers(&[])).is_err_and(|error| error.contains("no PT_LOAD")));
        assert!(judge("not json").is_err());
        assert!(judge("[]").is_err());
        assert!(judge(r#"[{"ProgramHeaders":[{"ProgramHeader":{"Type":{"Name":"PT_LOAD"}}}]}]"#).is_err_and(|error| error.contains("no alignment")));
    }

    #[test]
    fn a_tool_that_cannot_run_and_no_library_at_all_are_failures() {
        assert!(check(Path::new("/nonexistent/llvm-readelf"), Path::new("lib.so")).is_err_and(|error| error.contains("cannot run")));
        assert_eq!(run("/nonexistent/llvm-readelf", &[]), ExitCode::from(1));
        assert_eq!(run("/nonexistent/llvm-readelf", &["lib.so"]), ExitCode::from(1));
    }
}
