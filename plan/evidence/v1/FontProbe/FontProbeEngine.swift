import Foundation
import CoreText
import UIKit

/// EffectCraft's own font registration (effectcraft-text at the pinned revision, linked in as the
/// ecprobe static library), run on the folders sections 1-8 found.
extension FontProbe {

    /// Runs an ecprobe call and takes ownership of the string it returns.
    static func engine(_ call: () -> UnsafeMutablePointer<CChar>?) -> String {
        guard let p = call() else { return "<null>" }
        defer { ecprobe_free(p) }
        return String(cString: p)
    }

    /// Runs `body` and appends how long it took (AD2 runs at engine start, so the scan's cost matters).
    static func timed(_ body: () -> String) -> String {
        let clock = ContinuousClock()
        var out = ""
        let d = clock.measure { out = body() }
        return out + String(format: " (%.0f ms)", Double(d.components.attoseconds) / 1e15 + Double(d.components.seconds) * 1000)
    }

    static func engineCountFaces(_ dirs: [String]) -> String {
        timed { engine { dirs.joined(separator: "\n").withCString { ecprobe_count_faces($0) } } }
    }

    static func engineAddFontData(_ data: Data) -> String {
        data.withUnsafeBytes { buf in
            engine { ecprobe_add_font_data(buf.bindMemory(to: UInt8.self).baseAddress, buf.count) }
        }
    }

    // ---- 9. What add_source(&DirectorySource { dirs }) registers for each choice of dirs ----
    static func effectCraftRegistration(_ r: Report, coreText: CoreTextFonts) {
        r.line("--- 9. EffectCraft DirectorySource (effectcraft-text 10376058, linked into this app) ---")
        r.line("  DirectorySource::system() folders on this target (what scan_system() walks):")
        for l in engine({ ecprobe_system_dirs() }).split(separator: "\n") { r.line("    \(l)") }

        var cases: [(String, [String])] = [("[\(fontsRoot)]", [fontsRoot])]
        if let root = simulatorRoot {
            cases.append(("[<RuntimeRoot>\(fontsRoot)] (the iOS copy; no font files at its top level)", [root + fontsRoot]))
        }
        cases.append(("CoreText's \(coreText.dirs.count) font folders", coreText.dirs))
        // The walk goes 8 levels down, so a parent above the font folders would walk half the disk.
        let parent = commonParent(coreText.dirs)
        let parentIsFontFolder = (parent as NSString).pathComponents.contains("Fonts")
        if parentIsFontFolder { cases.append(("their common parent \(short(parent))", [parent])) }
        cases.append(("[one font file, \(coreText.files.first.map { short($0) } ?? "-")]", Array(coreText.files.prefix(1))))
        r.line("  DirectorySource { dirs }.faces() (lists, registers nothing):")
        for (label, dirs) in cases { r.line("    dirs = \(label): \(engineCountFaces(dirs))") }
        if !parentIsFontFolder { r.line("    (their common parent \(short(parent)) not walked: it is not inside a Fonts folder)") }
        // The walk takes every font file in a folder, also the ones CoreText does not offer the app.
        r.line("  per CoreText folder: files CoreText lists | what DirectorySource { dirs: [folder] }.faces() finds:")
        for dir in coreText.dirs {
            r.line("    \(short(dir)): \(coreText.filesByDir[dir]?.count ?? 0) | \(engineCountFaces([dir]))")
        }
        // FX-3's list_fonts rule: keep the EffectCraft faces whose (file, PostScript name) pair is one
        // CoreText lists (ecprobe's facematch, the same code as the host capture's, summary only).
        let ctFaces = coreText.faces.map { "\($0.postscript)\t\($0.file)\t\($0.family)" }.joined(separator: "\n")
        let match = coreText.dirs.joined(separator: "\n").withCString { dirs in
            ctFaces.withCString { ct in
                coreText.uiFamilies.joined(separator: "\n").withCString { ui in engine { ecprobe_face_match(dirs, ct, ui) } }
            }
        }
        r.line("  face-by-face match against CoreText's faces (ecprobe facematch, summary):")
        for l in match.split(separator: "\n") { r.line("    \(l)") }

        // Register CoreText's folders, then read families() the way list_fonts does.
        var out = ""
        let took = timed { out = engine { coreText.dirs.joined(separator: "\n").withCString { ecprobe_register_dirs($0) } }; return "" }
        let lines = out.split(separator: "\n").map(String.init)
        r.line("  add_source(CoreText's folders) + families()\(took): \(lines.first ?? "")")

        // The face each side draws a character with when the text's own font lacks it.
        r.line("  fallback for characters the default face lacks: EffectCraft fallback_for_locale | CoreText CTFontCreateForStringWithLanguage from the system UI font:")
        for sample in engine({ ecprobe_fallback_faces() }).split(separator: "\n") {
            let parts = sample.split(separator: "\t", maxSplits: 2).map(String.init)
            guard parts.count == 3 else { continue }
            let coreTextPick = systemUIFont().map { ui in
                describeFont(CTFontCreateForStringWithLanguage(ui, parts[0] as CFString, CFRange(location: 0, length: (parts[0] as NSString).length), parts[1] as CFString))
            } ?? "<no system UI font>"
            r.line("    \(parts[0]) (\(parts[1])): \(parts[2]) | \(coreTextPick)")
        }
        let engineFamilies = lines.dropFirst().map { line -> String in
            guard let cut = line.range(of: " [", options: .backwards) else { return line }
            return String(line[..<cut.lowerBound])
        }
        let lowered = Set(engineFamilies.map { $0.lowercased() })
        let missing = coreText.uiFamilies.filter { !lowered.contains($0.lowercased()) }
        r.line("  UIFont.familyNames found in families() (case-insensitive): \(coreText.uiFamilies.count - missing.count)/\(coreText.uiFamilies.count)")
        let uiLowered = Set(coreText.uiFamilies.map { $0.lowercased() })
        let notInUIKit = engineFamilies.filter { !$0.hasPrefix(".") && !uiLowered.contains($0.lowercased()) }
        r.line("  families() not dot-named and not in UIFont.familyNames (bundled and Arabic-named ones included): \(notInUIKit.count): \(notInUIKit.joined(separator: ", "))")
        r.line("  UIFont families not in families() under their English name: \(missing.count)\(missing.isEmpty ? "" : ": " + missing.joined(separator: ", "))")
        // What EffectCraft calls the faces in each such family's file (add_font_data, after the snapshot below).
        for family in missing {
            guard let name = UIFont.fontNames(forFamilyName: family).first,
                  let url = fileURL(CTFontCreateWithName(name as CFString, 12, nil)),
                  let data = try? Data(contentsOf: url, options: .alwaysMapped)
            else {
                r.line("    \(family): no readable file")
                continue
            }
            r.line("    \(family) is \(short(url.path)); EffectCraft reads it as \(engineAddFontData(data))")
        }
        r.line("  families() (\(engineFamilies.count), with style counts):")
        for l in lines.dropFirst() { r.line("    \(l)") }
        r.line()
    }

    /// The longest folder path every one of `dirs` lies in.
    static func commonParent(_ dirs: [String]) -> String {
        guard var common = dirs.first.map({ ($0 as NSString).pathComponents }) else { return fontsRoot }
        for d in dirs.dropFirst() {
            let c = (d as NSString).pathComponents
            common = Array(zip(common, c).prefix { $0 == $1 }.map(\.0))
        }
        return NSString.path(withComponents: common)
    }
}
