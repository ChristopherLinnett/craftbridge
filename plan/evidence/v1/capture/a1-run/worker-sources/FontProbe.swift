import Foundation
import Darwin
import CryptoKit

/// Probes whether a sandboxed iOS app can list /System/Library/Fonts,
/// enumerate its subfolders one level deep, and read font files in it.
/// Built for CraftBridge issue ChristopherLinnett/craftbridge#28 (seam S3 check).
enum FontProbe {

    static let fontsRoot = "/System/Library/Fonts"
    static let reportName = "fontprobe_report.txt"
    static var lastReport: String = "(probe not run yet)"

    // MARK: - Entry point (called from App.init)

    static func runAndReport() {
        let body = buildBody()

        // Write the report to Documents (section 6 describes the write itself).
        let fm = FileManager.default
        let docsURL = fm.urls(for: .documentDirectory, in: .userDomainMask).first
        var writeSection = "--- 6. Report file in Documents ---\n"
        writeSection += "  Documents dir: \(docsURL?.path ?? "<none resolved>")\n"
        let bodyData = Data((body + "\n").utf8)
        writeSection += "  Report body (sections 1-5): \(bodyData.count) bytes, SHA-256 \(sha256(bodyData))\n"

        var fileContents = body + "\n" + writeSection
        var stdoutOnlyExtra = ""

        if let docsURL {
            let dest = docsURL.appendingPathComponent(reportName)
            do {
                try fm.removeItem(at: dest) // start fresh each launch
            } catch {
                // nothing there yet, fine
            }
            do {
                fileContents += "  WRITE: OK -> \(dest.path)\n"
                let written = Data(fileContents.utf8)
                try written.write(to: dest)
                // Read it back and verify.
                let verdict: String
                do {
                    let back = try Data(contentsOf: dest)
                    verdict = back == written
                        ? "  READ-BACK: OK, file on disk matches this report (\(back.count) bytes)\n"
                        : "  READ-BACK: MISMATCH! read \(back.count) bytes\n"
                } catch {
                    verdict = "  READ-BACK: FAIL \(describe(error))\n"
                }
                fileContents += verdict
                // Append the verdict to the file so the Documents copy is
                // byte-identical to the stdout copy of this report.
                do {
                    let fh = try FileHandle(forWritingTo: dest)
                    fh.seekToEndOfFile()
                    fh.write(Data(verdict.utf8))
                    try fh.close()
                } catch {
                    fileContents += "  APPEND-VERDICT: FAIL \(describe(error)) (Documents copy lacks the last line)\n"
                }
            } catch {
                stdoutOnlyExtra = "  WRITE: FAIL \(describe(error))\n"
                fileContents += stdoutOnlyExtra
            }
        } else {
            fileContents += "  WRITE: FAIL no Documents directory resolved\n"
        }

        lastReport = fileContents

        // stdout (captured by `simctl launch --console-pty`)
        FileHandle.standardOutput.write(Data(fileContents.utf8))
        fflush(stdout)
        // os_log (visible via `simctl spawn <device> log show`; NSLog lands in os_log)
        NSLog("FontProbe report begin\n%@\nFontProbe report end", fileContents)
    }

    // MARK: - Report body

    private static func buildBody() -> String {
        let fm = FileManager.default
        var out: [String] = []
        func line(_ s: String = "") { out.append(s) }

        line("=== CraftBridge FontProbe report (issue craftbridge#28, seam S3) ===")
        line("Timestamp : \(ISO8601DateFormatter().string(from: Date()))")
        line("OS        : \(ProcessInfo.processInfo.operatingSystemVersionString)")
        line("Process   : \(ProcessInfo.processInfo.processName) (pid \(ProcessInfo.processInfo.processIdentifier))")
        line("App bundle: \(Bundle.main.bundlePath)")
        line("Target path: \(fontsRoot)")
        line()

        // ---- 1. Provenance: what does the app's path actually resolve to ----
        line("--- 1. Path provenance ---")
        var resolved: String?
        if let c = realpath(fontsRoot, nil) {
            resolved = String(cString: c)
            free(c)
        }
        line("  realpath(\(fontsRoot)) = \(resolved ?? "<FAILED>")")
        var st = stat()
        if lstat(fontsRoot, &st) == 0 {
            let fmt = Int(st.st_mode) & Int(S_IFMT)
            let kind: String
            if fmt == Int(S_IFLNK) { kind = "symlink" }
            else if fmt == Int(S_IFDIR) { kind = "directory" }
            else { kind = "mode 0o\(String(fmt, radix: 8))" }
            line("  lstat   : \(kind), st_dev=\(st.st_dev)")
        } else {
            line("  lstat   : FAILED errno \(errno) (\(String(cString: strerror(errno))))")
        }
        line("  Environment of this process (all, values truncated to 160 chars):")
        for key in ProcessInfo.processInfo.environment.keys.sorted() {
            let value = ProcessInfo.processInfo.environment[key] ?? ""
            line("    \(key) = \(String(value.prefix(160)))")
        }
        line()

        // ---- 2. List top level ----
        line("--- 2. List \(fontsRoot) (top level) ---")
        var topFiles: [String] = []
        var subDirs: [String] = []
        var extCounts: [String: Int] = [:]
        var totalFiles = 0

        func bump(_ path: String) {
            let ext = (path as NSString).pathExtension.lowercased()
            extCounts[ext.isEmpty ? "(no extension)" : "." + ext, default: 0] += 1
            totalFiles += 1
        }

        do {
            let entries = try fm.contentsOfDirectory(atPath: fontsRoot).sorted()
            for name in entries {
                let p = fontsRoot + "/" + name
                var isDir: ObjCBool = false
                _ = fm.fileExists(atPath: p, isDirectory: &isDir)
                if isDir.boolValue { subDirs.append(p) } else { topFiles.append(p) }
            }
            line("  OK: \(entries.count) entries = \(topFiles.count) files + \(subDirs.count) subfolders")
            for f in topFiles { line("    file     \(f)") }
            for d in subDirs { line("    subfolder \(d)") }
            for f in topFiles { bump(f) }
        } catch {
            line("  FAIL: \(describe(error))")
        }
        line()

        // ---- 3. One level deep ----
        line("--- 3. Subfolders of \(fontsRoot), one level deep ---")
        if subDirs.isEmpty { line("  (no subfolders found)") }
        for d in subDirs {
            do {
                let entries = try fm.contentsOfDirectory(atPath: d).sorted()
                line("  OK: \(d) -> \(entries.count) entries")
                for name in entries {
                    let p = d + "/" + name
                    var isDir: ObjCBool = false
                    _ = fm.fileExists(atPath: p, isDirectory: &isDir)
                    if isDir.boolValue {
                        line("    dir (not descended, one-level limit) \(p)")
                    } else {
                        line("    file \(p)")
                        bump(p)
                    }
                }
            } catch {
                line("  FAIL \(d): \(describe(error))")
            }
        }
        line()

        // ---- 4. Counts by extension ----
        line("--- 4. File counts by extension (top level + one level deep) ---")
        let sorted = extCounts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }
        if sorted.isEmpty { line("  (none)") }
        for (ext, n) in sorted { line("  \(ext) : \(n)") }
        line("  TOTAL files: \(totalFiles)")
        line()

        // ---- 5. Read first 4 bytes of one font per kind ----
        line("--- 5. Font reads: first 4 bytes, one per kind (.ttc/.ttf/.otf, first match by name) ---")
        var subFiles: [String] = []
        for d in subDirs {
            let entries = ((try? fm.contentsOfDirectory(atPath: d)) ?? []).sorted()
            for name in entries {
                let p = d + "/" + name
                var isDir: ObjCBool = false
                _ = fm.fileExists(atPath: p, isDirectory: &isDir)
                if !isDir.boolValue { subFiles.append(p) }
            }
        }
        let allFiles = topFiles + subFiles
        for kind in ["ttc", "ttf", "otf"] {
            guard let pick = allFiles.first(where: { $0.lowercased().hasSuffix("." + kind) }) else {
                line("  [\(kind.uppercased())] SKIP: no .\(kind) file present")
                continue
            }
            do {
                let fh = try FileHandle(forReadingFrom: URL(fileURLWithPath: pick))
                let data = try fh.read(upToCount: 4) ?? Data()
                try fh.close()
                let hex = data.map { String(format: "%02X", $0) }.joined(separator: " ")
                let ascii = String(decoding: data.map { (0x20...0x7E).contains($0) ? $0 : UInt8(0x2E) }, as: UTF8.self)
                line("  [\(kind.uppercased())] OK: \(pick)")
                line("        read \(data.count) bytes: hex [\(hex)] ascii \"\(ascii)\"")
            } catch {
                line("  [\(kind.uppercased())] FAIL: \(pick): \(describe(error))")
            }
        }
        line()
        return out.joined(separator: "\n")
    }

    // MARK: - Helpers

    private static func describe(_ error: Error) -> String {
        let ns = error as NSError
        return "\(ns.domain) code \(ns.code): \(ns.localizedDescription)"
    }

    private static func sha256(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}
