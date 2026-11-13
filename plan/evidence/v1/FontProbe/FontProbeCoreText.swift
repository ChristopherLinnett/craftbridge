import Foundation
import CoreText
import UIKit

/// The fonts CoreText reports, where their files are, and whether the app can read them.
extension FontProbe {

    /// What section 8 found, for the EffectCraft section.
    struct CoreTextFonts {
        /// Distinct font files behind CoreText's available fonts.
        var files: [String] = []
        /// Their parent folders, distinct and sorted.
        var dirs: [String] = []
        /// The files in each of `dirs`.
        var filesByDir: [String: [String]] = [:]
        var uiFamilies: [String] = []
        /// One entry per descriptor: PostScript name, file and family ("-" where absent), sorted;
        /// the identity FX-3's face-by-face `list_fonts` rule matches on (section 9).
        var faces: [(postscript: String, file: String, family: String)] = []
    }

    /// The file a font is loaded from.
    static func fileURL(_ font: CTFont) -> URL? {
        CTFontCopyAttribute(font, kCTFontURLAttribute) as? URL
    }

    // ---- 8. CoreText's available fonts ----
    static func coreTextFonts(_ r: Report) -> CoreTextFonts {
        var result = CoreTextFonts()
        r.line("--- 8. CoreText: CTFontCollectionCreateFromAvailableFonts + kCTFontURLAttribute ---")
        r.line("  (CTFontManagerCopyAvailableFontURLs is macOS-only: the iOS SDK marks it CT_UNAVAILABLE(ios))")
        let collection = CTFontCollectionCreateFromAvailableFonts(nil)
        let descriptors = (CTFontCollectionCreateMatchingFontDescriptors(collection) as? [CTFontDescriptor]) ?? []
        var withoutURL: [String] = []
        var files = Set<String>()
        var ctFamilies = Set<String>()
        for d in descriptors {
            let family = CTFontDescriptorCopyAttribute(d, kCTFontFamilyNameAttribute) as? String
            if let family { ctFamilies.insert(family) }
            let postscript = CTFontDescriptorCopyAttribute(d, kCTFontNameAttribute) as? String
            let url = CTFontDescriptorCopyAttribute(d, kCTFontURLAttribute) as? URL
            if let url {
                files.insert(url.path)
            } else {
                withoutURL.append(postscript ?? "?")
            }
            result.faces.append((postscript ?? "-", url?.path ?? "-", family ?? "-"))
        }
        result.faces.sort { ($0.postscript, $0.file, $0.family) < ($1.postscript, $1.file, $1.family) }
        result.files = files.sorted()
        result.filesByDir = Dictionary(grouping: result.files) { ($0 as NSString).deletingLastPathComponent }
        result.dirs = result.filesByDir.keys.sorted()
        r.line("  descriptors: \(descriptors.count); with a URL: \(descriptors.count - withoutURL.count); without: \(withoutURL.count)")
        for n in withoutURL.prefix(20) { r.line("    no URL: \(n)") }
        r.line("  distinct font files: \(result.files.count), in \(result.dirs.count) folders:")
        for dir in result.dirs {
            let names = (result.filesByDir[dir] ?? []).map { ($0 as NSString).lastPathComponent }
            r.line("    \(names.count) in \(short(dir)): \(names.joined(separator: ", "))")
        }
        if let root = simulatorRoot {
            let inRuntime = result.files.filter { $0.hasPrefix(root) }.count
            let inMacFolder = result.files.filter { $0.hasPrefix(fontsRoot + "/") }.count
            r.line("  under the runtime root: \(inRuntime); under the Mac's \(fontsRoot): \(inMacFolder); elsewhere: \(result.files.count - inRuntime - inMacFolder)")
        }

        // Can the app read the bytes at every URL?
        var ok = 0
        var failures: [String] = []
        var magic: [String: Int] = [:]
        var bytes: Int64 = 0
        for p in result.files {
            do {
                let head = try readHead(p)
                ok += 1
                magic[describeMagic(head), default: 0] += 1
                bytes += ((try? FileManager.default.attributesOfItem(atPath: p)[.size]) as? NSNumber)?.int64Value ?? 0
            } catch {
                failures.append("\(short(p)): \(describe(error))")
            }
        }
        r.line("  read first 4 bytes of every file: OK \(ok), FAIL \(failures.count); total size of the readable files \(bytes / 1_048_576) MiB")
        for f in failures.prefix(20) { r.line("    FAIL \(f)") }
        for (m, n) in magic.sorted(by: { $0.value > $1.value }) { r.line("    \(n) x \(m)") }

        // Families as UIKit and CoreText list them.
        result.uiFamilies = UIFont.familyNames.sorted()
        let ctManagerFamilies = (CTFontManagerCopyAvailableFontFamilyNames() as? [String]) ?? []
        r.line("  UIFont.familyNames: \(result.uiFamilies.count); CTFontManagerCopyAvailableFontFamilyNames: \(ctManagerFamilies.count); families in the collection: \(ctFamilies.count) (\(ctFamilies.filter { $0.hasPrefix(".") }.count) hidden, named with a leading dot)")
        for (label, names) in [("UIFont.familyNames", result.uiFamilies), ("CTFontManagerCopyAvailableFontFamilyNames", ctManagerFamilies)] {
            let differ = Set(names).symmetricDifference(ctFamilies).sorted()
            r.line("  \(label) and the collection's families differ in: \(differ.count)\(differ.isEmpty ? "" : " " + differ.prefix(10).joined(separator: ", "))")
        }

        // Where the fonts an app asks for by name, and the system UI font, live.
        r.line("  font files behind named fonts (CTFontCopyAttribute kCTFontURLAttribute):")
        for (label, font) in sampleFonts() {
            r.line("    \(label) -> \(describeFont(font))")
        }
        // Every face's identity, and UIKit's family list, as tab-separated lines a script can read.
        r.line("  faces, one per descriptor (face<TAB>PostScript name<TAB>file<TAB>family; - where absent):")
        for f in result.faces { r.line("    face\t\(f.postscript)\t\(short(f.file))\t\(f.family)") }
        r.line("  UIFont.familyNames, one per line (uifamily<TAB>name):")
        for n in result.uiFamilies { r.line("    uifamily\t\(n)") }
        r.line()
        return result
    }

    /// "<PostScript name> at <file>".
    static func describeFont(_ font: CTFont) -> String {
        "\(CTFontCopyPostScriptName(font)) at \(fileURL(font).map { short($0.path) } ?? "<no URL>")"
    }

    static func systemUIFont() -> CTFont? {
        CTFontCreateUIFontForLanguage(.system, 17, nil)
    }

    /// The system UI font and a few named fonts of each kind (TrueType, CFF, collection, colour).
    static func sampleFonts() -> [(String, CTFont)] {
        var out: [(String, CTFont)] = []
        if let ui = systemUIFont() { out.append(("system UI font", ui)) }
        for name in ["Helvetica", "Georgia", "PingFangSC-Regular", "AppleColorEmoji", "Noteworthy-Light"] {
            let f = CTFontCreateWithName(name as CFString, 17, nil)
            // CoreText substitutes a fallback for an unknown name; keep only the font asked for.
            if CTFontCopyPostScriptName(f) as String == name { out.append((name, f)) }
        }
        return out
    }

    // ---- 10. Font data without file access: CTFontCopyAvailableTables / CTFontCopyTable ----
    static func tablesWithoutFileAccess(_ r: Report) {
        r.line("--- 10. Font data from CoreText tables (no file access) ---")
        for (label, font) in sampleFonts() {
            let tags = availableTables(font)
            var tables: [(CTFontTableTag, Data)] = []
            for t in tags {
                if let d = CTFontCopyTable(font, t, []) as Data? { tables.append((t, d)) }
            }
            let rebuilt = sfnt(tables)
            let descs = (CTFontManagerCreateFontDescriptorsFromData(rebuilt as CFData) as? [CTFontDescriptor]) ?? []
            let family = descs.first.flatMap { CTFontDescriptorCopyAttribute($0, kCTFontFamilyNameAttribute) as? String } ?? "<none>"
            r.line("  \(label): \(tags.count) tables, \(tables.count) copied, \(rebuilt.count) bytes rebuilt; CoreText reloads it as \(descs.count) face(s), family \"\(family)\" (original \"\(CTFontCopyFamilyName(font))\")")
            r.line("    file comparison: \(compareWithFile(font, tables: tables))")
            r.line("    EffectCraft add_font_data(rebuilt): \(engineAddFontData(rebuilt))")
        }
        r.line()
    }

    static func availableTables(_ font: CTFont) -> [CTFontTableTag] {
        // The array holds unboxed tag values, not CFNumbers.
        guard let arr = CTFontCopyAvailableTables(font, []) else { return [] }
        return (0..<CFArrayGetCount(arr)).map { i in
            CTFontTableTag(UInt(bitPattern: CFArrayGetValueAtIndex(arr, i)))
        }
    }

    /// An sfnt font file holding `tables` (sorted table directory, 4-byte aligned tables).
    static func sfnt(_ tables: [(CTFontTableTag, Data)]) -> Data {
        let sorted = tables.sorted { $0.0 < $1.0 }
        let cff = sorted.contains { $0.0 == 0x4346_4620 || $0.0 == 0x4346_4632 } // 'CFF ', 'CFF2'
        var out = Data()
        func u32(_ v: UInt32) { out.append(contentsOf: withUnsafeBytes(of: v.bigEndian, Array.init)) }
        func u16(_ v: UInt16) { out.append(contentsOf: withUnsafeBytes(of: v.bigEndian, Array.init)) }
        let n = sorted.count
        var pow2 = 1, log2 = 0
        while pow2 * 2 <= n { pow2 *= 2; log2 += 1 }
        u32(cff ? 0x4F54_544F : 0x0001_0000)
        u16(UInt16(n)); u16(UInt16(pow2 * 16)); u16(UInt16(log2)); u16(UInt16(n * 16 - pow2 * 16))
        var offset = 12 + 16 * n
        for (tag, data) in sorted {
            u32(tag); u32(checksum(data)); u32(UInt32(offset)); u32(UInt32(data.count))
            offset += (data.count + 3) & ~3
        }
        for (_, data) in sorted {
            out.append(data)
            out.append(contentsOf: [UInt8](repeating: 0, count: ((data.count + 3) & ~3) - data.count))
        }
        return out
    }

    /// The sfnt table checksum: the sum of the table's big-endian 32-bit words, zero-padded.
    static func checksum(_ data: Data) -> UInt32 {
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) -> UInt32 in
            let b = raw.bindMemory(to: UInt8.self)
            var sum: UInt32 = 0
            var i = 0
            while i + 4 <= b.count {
                sum = sum &+ (UInt32(b[i]) << 24 | UInt32(b[i + 1]) << 16 | UInt32(b[i + 2]) << 8 | UInt32(b[i + 3]))
                i += 4
            }
            var tail: UInt32 = 0
            for k in 0..<4 { tail = tail << 8 | UInt32(i + k < b.count ? b[i + k] : 0) }
            return sum &+ tail
        }
    }

    /// Whether CoreText's tables are the bytes in the font's file: finds the face in the file (a
    /// collection holds several) whose `name` table matches, then compares every table.
    static func compareWithFile(_ font: CTFont, tables: [(CTFontTableTag, Data)]) -> String {
        guard let url = fileURL(font) else { return "no URL" }
        guard let file = try? Data(contentsOf: url, options: .alwaysMapped) else { return "file not readable" }
        func u32(_ at: Int) -> UInt32 {
            guard at + 4 <= file.count else { return 0 }
            return file[at..<at + 4].reduce(0) { $0 << 8 | UInt32($1) }
        }
        let faceOffsets: [Int] = u32(0) == 0x7474_6366 // 'ttcf'
            ? (0..<Int(u32(8))).map { Int(u32(12 + 4 * $0)) }
            : [0]
        let ct = Dictionary(tables, uniquingKeysWith: { a, _ in a })
        for (index, base) in faceOffsets.enumerated() {
            let n = Int(u32(base + 4) >> 16)
            var inFile: [CTFontTableTag: Data] = [:]
            for i in 0..<n {
                let rec = base + 12 + 16 * i
                let off = Int(u32(rec + 8)), len = Int(u32(rec + 12))
                guard off + len <= file.count else { continue }
                inFile[u32(rec)] = file[off..<off + len]
            }
            guard let name = inFile[0x6E61_6D65], name == ct[0x6E61_6D65] else { continue } // 'name'
            let same = inFile.filter { ct[$0.key] == $0.value }.count
            let missing = Set(inFile.keys).subtracting(ct.keys).map(tagString).sorted()
            let extra = Set(ct.keys).subtracting(inFile.keys).map(tagString).sorted()
            return "face \(index) of \(faceOffsets.count) in \(url.lastPathComponent) (\(file.count) bytes): \(same)/\(inFile.count) tables byte-identical; missing from CoreText \(missing); only in CoreText \(extra)"
        }
        return "no face in \(url.lastPathComponent) has CoreText's name table"
    }

    static func tagString(_ t: CTFontTableTag) -> String {
        String(decoding: [24, 16, 8, 0].map { UInt8(truncatingIfNeeded: t >> $0) }, as: UTF8.self)
    }
}
