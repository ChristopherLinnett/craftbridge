import Foundation
import Darwin
import CryptoKit

/// Probes what an iOS app sees of the system fonts: the /System/Library/Fonts folder through
/// plain file access, the fonts CoreText reports (FontProbeCoreText.swift), and what
/// EffectCraft's own font registration finds in each (FontProbeEngine.swift).
enum FontProbe {

    static let fontsRoot = "/System/Library/Fonts"
    static let reportName = "fontprobe_report.txt"
    /// A file the capture script puts on the Mac outside every app container. Reading it shows
    /// whether the app's file access is confined to an iOS sandbox; a device never has it.
    static let canaryPath = "/tmp/craftbridge-v1/capture/sandbox-canary.txt"
    /// The simulator runtime's root (the iOS system the simulator runs); unset on a device.
    static let simulatorRoot = ProcessInfo.processInfo.environment["IPHONE_SIMULATOR_ROOT"]
    static var lastReport: String = "(probe not run yet)"

    /// Report lines, shared by every section. Each line also goes to stdout as it is made, so a
    /// section that hangs or crashes still leaves the lines before it.
    final class Report {
        private(set) var lines: [String] = []
        func line(_ s: String = "") {
            lines.append(s)
            FileHandle.standardOutput.write(Data((s + "\n").utf8))
        }
        var text: String { lines.joined(separator: "\n") }
    }

    // MARK: - Entry point (called from App.init)

    static func runAndReport() {
        let body = buildBody()

        // Write the report to Documents (the last section describes the write itself).
        let fm = FileManager.default
        let docsURL = fm.urls(for: .documentDirectory, in: .userDomainMask).first
        var writeSection = "--- 11. Report file in Documents ---\n"
        writeSection += "  Documents dir: \(docsURL?.path ?? "<none resolved>")\n"
        let bodyData = Data((body + "\n").utf8)
        writeSection += "  Report body (sections 1-10): \(bodyData.count) bytes, SHA-256 \(sha256(bodyData))\n"

        var fileContents = body + "\n" + writeSection

        if let docsURL {
            let dest = docsURL.appendingPathComponent(reportName)
            do {
                fileContents += "  WRITE: OK -> \(dest.path)\n"
                let written = Data(fileContents.utf8)
                try written.write(to: dest)
                let verdict: String
                do {
                    let back = try Data(contentsOf: dest)
                    verdict = back == written
                        ? "  READ-BACK: OK, file on disk matches this report (\(back.count) bytes before this line)\n"
                        : "  READ-BACK: MISMATCH! read \(back.count) bytes\n"
                } catch {
                    verdict = "  READ-BACK: FAIL \(describe(error))\n"
                }
                fileContents += verdict
                // Append the verdict so the Documents copy is byte-identical to stdout.
                do {
                    let fh = try FileHandle(forWritingTo: dest)
                    try fh.seekToEnd()
                    try fh.write(contentsOf: Data(verdict.utf8))
                    try fh.close()
                } catch {
                    fileContents += "  APPEND-VERDICT: FAIL \(describe(error)) (Documents copy lacks the last line)\n"
                }
            } catch {
                fileContents += "  WRITE: FAIL \(describe(error))\n"
            }
        } else {
            fileContents += "  WRITE: FAIL no Documents directory resolved\n"
        }

        lastReport = fileContents

        // stdout (captured by `simctl launch --stdout=` or `devicectl ... --console`) already has the
        // body line by line; the Documents section completes it.
        FileHandle.standardOutput.write(Data(fileContents.utf8.dropFirst(body.utf8.count + 1)))
        // os_log keeps only the first ~1 KB of a message, so it gets a pointer, not the report.
        NSLog("FontProbe report: %zd bytes, written to Documents/%@", fileContents.utf8.count, reportName)
    }

    // MARK: - Report body

    private static func buildBody() -> String {
        let r = Report()
        r.line("=== CraftBridge FontProbe report (issue craftbridge#28, V1 / AD2) ===")
        r.line("Timestamp : \(ISO8601DateFormatter().string(from: Date()))")
        r.line("OS        : \(ProcessInfo.processInfo.operatingSystemVersionString)")
        r.line("Process   : \(ProcessInfo.processInfo.processName) (pid \(ProcessInfo.processInfo.processIdentifier))")
        r.line("App bundle: \(Bundle.main.bundlePath)")
        r.line("Simulator : \(simulatorRoot.map { "yes, runtime root \($0)" } ?? "no (IPHONE_SIMULATOR_ROOT unset)")")
        r.line()

        provenance(r)
        let tree = listFontsRoot(r)
        readSamples(r, files: tree.allFiles)
        runtimeOnlyPath(r)
        sandboxCanary(r)
        let coreText = coreTextFonts(r)
        effectCraftRegistration(r, coreText: coreText)
        tablesWithoutFileAccess(r)
        return r.text
    }

    // ---- 1. Provenance: which file system the app's path lands on ----
    private static func provenance(_ r: Report) {
        r.line("--- 1. Path provenance ---")
        var resolved: String?
        if let c = realpath(fontsRoot, nil) {
            resolved = String(cString: c)
            free(c)
        }
        r.line("  realpath(\(fontsRoot)) = \(resolved ?? "<FAILED>")")
        let appDev = describeDevice(fontsRoot, as: "app's \(fontsRoot)")
        r.line(appDev.text)
        if let root = simulatorRoot {
            let runtimeDev = describeDevice(root + fontsRoot, as: "runtime root's copy")
            r.line(runtimeDev.text)
            if let a = appDev.device, let b = runtimeDev.device {
                r.line("  same volume as the runtime root's copy: \(a == b ? "YES" : "NO")")
            }
        }
        r.line("  Environment of this process (all, values truncated to 160 chars):")
        let env = ProcessInfo.processInfo.environment
        for key in env.keys.sorted() {
            r.line("    \(key) = \(String((env[key] ?? "").prefix(160)))")
        }
        r.line()
    }

    private static func describeDevice(_ path: String, as label: String) -> (text: String, device: dev_t?) {
        var st = stat()
        guard lstat(path, &st) == 0 else {
            return ("  lstat \(label): FAILED errno \(errno) (\(String(cString: strerror(errno))))", nil)
        }
        let fmt = st.st_mode & S_IFMT
        let kind = fmt == S_IFLNK ? "symlink" : fmt == S_IFDIR ? "directory" : "mode 0o\(String(fmt, radix: 8))"
        return ("  lstat \(label): \(kind), st_dev=\(st.st_dev)", st.st_dev)
    }

    // ---- 2-4. List the folder, its subfolders one level deep, count by extension ----
    private struct FontTree {
        var topFiles: [String] = []
        var subDirs: [String] = []
        var subFiles: [String] = []
        var allFiles: [String] { topFiles + subFiles }
    }

    private static func listFontsRoot(_ r: Report) -> FontTree {
        var tree = FontTree()
        r.line("--- 2. List \(fontsRoot) (top level) ---")
        do {
            let top = try list(fontsRoot)
            tree.topFiles = top.files
            tree.subDirs = top.dirs
            r.line("  OK: \(top.files.count + top.dirs.count) entries = \(top.files.count) files + \(top.dirs.count) subfolders")
            for f in top.files { r.line("    file     \(f)") }
            for d in top.dirs { r.line("    subfolder \(d)") }
        } catch {
            r.line("  FAIL: \(describe(error))")
        }
        r.line()

        r.line("--- 3. Subfolders of \(fontsRoot), one level deep ---")
        if tree.subDirs.isEmpty { r.line("  (no subfolders found)") }
        for d in tree.subDirs {
            do {
                let sub = try list(d)
                r.line("  OK: \(d) -> \(sub.files.count + sub.dirs.count) entries")
                for p in sub.dirs { r.line("    dir (not descended, one-level limit) \(p)") }
                for p in sub.files { r.line("    file \(p)") }
                tree.subFiles += sub.files
            } catch {
                r.line("  FAIL \(d): \(describe(error))")
            }
        }
        r.line()

        r.line("--- 4. File counts by extension (top level + one level deep) ---")
        var extCounts: [String: Int] = [:]
        for p in tree.allFiles {
            let ext = (p as NSString).pathExtension.lowercased()
            extCounts[ext.isEmpty ? "(no extension)" : "." + ext, default: 0] += 1
        }
        let sorted = extCounts.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }
        if sorted.isEmpty { r.line("  (none)") }
        for (ext, n) in sorted { r.line("  \(ext) : \(n)") }
        r.line("  TOTAL files: \(tree.allFiles.count)")
        r.line()
        return tree
    }

    // ---- 5. Read the first bytes of one font per kind ----
    private static func readSamples(_ r: Report, files: [String]) {
        r.line("--- 5. Font reads: first 4 bytes, one per kind (.ttc/.ttf/.otf, first match by name) ---")
        for kind in ["ttc", "ttf", "otf"] {
            guard let pick = files.first(where: { $0.lowercased().hasSuffix("." + kind) }) else {
                r.line("  [\(kind.uppercased())] SKIP: no .\(kind) file present")
                continue
            }
            do {
                let head = try readHead(pick)
                r.line("  [\(kind.uppercased())] OK: \(pick)")
                r.line("        read \(head.count) bytes: \(describeMagic(head))")
            } catch {
                r.line("  [\(kind.uppercased())] FAIL: \(pick): \(describe(error))")
            }
        }
        r.line()
    }

    // ---- 6. A folder only the iOS layout has ----
    private static func runtimeOnlyPath(_ r: Report) {
        let core = fontsRoot + "/Core"
        r.line("--- 6. \(core) (exists in the iOS font layout, not in the macOS one) ---")
        do {
            let l = try list(core)
            r.line("  OK: \(l.files.count) files, \(l.dirs.count) folders")
        } catch {
            r.line("  FAIL: \(describe(error))")
        }
        r.line()
    }

    // ---- 7. Is file access confined to an iOS sandbox? ----
    private static func sandboxCanary(_ r: Report) {
        r.line("--- 7. Sandbox canary (a Mac file outside every app container) ---")
        do {
            let text = String(decoding: try Data(contentsOf: URL(fileURLWithPath: canaryPath)), as: UTF8.self)
            r.line("  READ \(canaryPath): OK \"\(text.trimmingCharacters(in: .whitespacesAndNewlines))\"")
            r.line("  -> this process reads arbitrary Mac files: no iOS sandbox confines its file access")
        } catch {
            r.line("  READ \(canaryPath): \(describe(error))")
        }
        r.line()
    }

    // MARK: - Helpers shared by every section

    struct Listing {
        var files: [String] = []
        var dirs: [String] = []
    }

    /// Full paths of a folder's entries, sorted, split into files and folders.
    static func list(_ dir: String) throws -> Listing {
        let fm = FileManager.default
        var out = Listing()
        for name in try fm.contentsOfDirectory(atPath: dir).sorted() {
            let p = (dir as NSString).appendingPathComponent(name)
            var isDir: ObjCBool = false
            _ = fm.fileExists(atPath: p, isDirectory: &isDir)
            if isDir.boolValue { out.dirs.append(p) } else { out.files.append(p) }
        }
        return out
    }

    /// The first `count` bytes of a file.
    static func readHead(_ path: String, count: Int = 4) throws -> Data {
        let fh = try FileHandle(forReadingFrom: URL(fileURLWithPath: path))
        defer { try? fh.close() }
        return try fh.read(upToCount: count) ?? Data()
    }

    static func describeMagic(_ data: Data) -> String {
        let hex = data.map { String(format: "%02X", $0) }.joined(separator: " ")
        let ascii = String(decoding: data.map { (0x20...0x7E).contains($0) ? $0 : UInt8(0x2E) }, as: UTF8.self)
        return "hex [\(hex)] ascii \"\(ascii)\""
    }

    /// A path with the simulator runtime root shown as `<RuntimeRoot>`.
    static func short(_ path: String) -> String {
        guard let root = simulatorRoot, path.hasPrefix(root) else { return path }
        return "<RuntimeRoot>" + path.dropFirst(root.count)
    }

    static func describe(_ error: Error) -> String {
        let ns = error as NSError
        var s = "\(ns.domain) code \(ns.code): \(ns.localizedDescription)"
        if let u = ns.userInfo[NSUnderlyingErrorKey] as? NSError {
            s += " [underlying \(u.domain) code \(u.code)]"
        }
        return s
    }

    static func sha256(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}
