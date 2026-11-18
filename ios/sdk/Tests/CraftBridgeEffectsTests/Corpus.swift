// The conformance corpus as Swift reads it: `conformance/effects/corpus.toml`, the one file every runner
// reads, parsed by TOMLDecoder and never copied, so a scenario is written once. The model is strict, as
// the desktop runner's is: a key or a kind the runner does not read is refused, never ignored, so a check
// the corpus gains cannot go unrun here.

import CraftBridge
import Foundation
import TOMLDecoder

/// A key of any table, so the table's own keys can be listed.
private struct AnyKey: CodingKey {
    let stringValue: String
    let intValue: Int? = nil

    init?(stringValue: String) { self.stringValue = stringValue }

    init?(intValue: Int) { nil }
}

extension Decoder {
    /// The container of the table being read, once every key it holds is one of `known`.
    fileprivate func table<Key: CodingKey>(_ keys: Key.Type, allowing known: Set<String>) throws
        -> KeyedDecodingContainer<Key>
    {
        let held = try container(keyedBy: AnyKey.self).allKeys.map(\.stringValue)
        if let unknown = held.filter({ !known.contains($0) }).min() {
            throw DecodingError.dataCorrupted(
                .init(codingPath: codingPath, debugDescription: "the runner does not read the key `\(unknown)`")
            )
        }
        return try container(keyedBy: keys)
    }
}

/// The keys a table's own `CodingKeys` name.
private func names<Key: CaseIterable & CodingKey>(_ keys: Key.Type) -> Set<String> {
    Set(keys.allCases.map(\.stringValue))
}

extension Corpus {
    /// The corpus file the desktop runner reads, found from where the compiler read this source, which is
    /// the repository on the machine that builds the tests, the simulator's host.
    static let file = URL(filePath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appending(path: "conformance/effects/corpus.toml")
}

/// Which sides run a scenario.
enum Sides: String, Decodable, Sendable {
    case both
    case oracle
    case plugin

    /// Whether the plug-in side, the one a phone runs, takes part.
    var runsPlugin: Bool { self != .oracle }
}

/// The corpus: its scenarios, in the order the file has them.
struct Corpus: Decodable, Sendable {
    let scenarios: [Scenario]

    private enum CodingKeys: String, CodingKey, CaseIterable { case scenario }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        scenarios = try table.decode([Scenario].self, forKey: .scenario)
    }

    /// The corpus `text` holds, parsed as TOML.
    init(toml text: String) throws {
        let json = try JSONValue(toml: text).jsonText()
        self = try JSONDecoder().decode(Corpus.self, from: Data(json.utf8))
    }

    /// The corpus in `file`.
    init(file: URL) throws {
        try self.init(toml: try String(contentsOf: file, encoding: .utf8))
    }
}

extension JSONValue {
    /// The JSON value a TOML document holds. TOMLDecoder reads the document and gives each value its
    /// Swift type; a date or time, which JSON has no value for, is refused, the corpus having none.
    fileprivate init(toml text: String) throws {
        self = try JSONValue(tomlValue: try [String: Any](TOMLTable(source: text)))
    }

    private init(tomlValue value: Any) throws {
        switch value {
        case let value as Bool: self = .bool(value)
        case let value as Int64: self = .number(Double(value))
        case let value as Double: self = .number(value)
        case let value as String: self = .string(value)
        case let value as [Any]: self = .array(try value.map { try JSONValue(tomlValue: $0) })
        case let value as [String: Any]: self = .object(try value.mapValues { try JSONValue(tomlValue: $0) })
        default: throw CraftError.InvalidParams(detail: "the corpus holds a TOML value JSON has none for: \(value)")
        }
    }
}

/// One scenario: its steps, and what to check after them.
struct Scenario: Decodable, Sendable {
    /// The plan's identifier (`T5`), which every failure names.
    let id: String
    let title: String
    let sides: Sides
    /// Files put in the folder before the first step.
    let fixtures: [Fixture]
    let steps: [Step]
    /// Values the step outputs must hold.
    let expects: [Expect]
    /// Outputs the two sides must print alike: the oracle's, so held on the desktop.
    let sames: [Same]
    /// Files the sides must hold alike, the oracle's against the plug-in's: held on the desktop.
    let compares: [Compare]
    /// The names the folders must hold.
    let folders: [Folder]

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case id, title, sides, fixture, step, expect, same, compare, folder
    }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        id = try table.decode(String.self, forKey: .id)
        title = try table.decode(String.self, forKey: .title)
        sides = try table.decodeIfPresent(Sides.self, forKey: .sides) ?? .both
        fixtures = try table.decodeIfPresent([Fixture].self, forKey: .fixture) ?? []
        steps = try table.decodeIfPresent([Step].self, forKey: .step) ?? []
        expects = try table.decodeIfPresent([Expect].self, forKey: .expect) ?? []
        sames = try table.decodeIfPresent([Same].self, forKey: .same) ?? []
        compares = try table.decodeIfPresent([Compare].self, forKey: .compare) ?? []
        folders = try table.decodeIfPresent([Folder].self, forKey: .folder) ?? []
    }
}

/// A file a scenario starts with.
struct Fixture: Decodable, Sendable {
    /// Relative to the corpus file's folder.
    let from: String
    /// Relative to the scenario's folder.
    let to: String
    /// The file's text says `<container>` where the scenario's folder goes.
    let namesContainer: Bool

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case from, to
        case namesContainer = "names_container"
    }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        from = try table.decode(String.self, forKey: .from)
        to = try table.decode(String.self, forKey: .to)
        namesContainer = try table.decodeIfPresent(Bool.self, forKey: .namesContainer) ?? false
    }
}

/// How the plug-in's app is built for one step.
struct Host: Decodable, Sendable {
    /// The plug-in folder the app names, relative to the scenario's folder; plug-ins are off without one.
    let plugins: String?
    /// The app is handed the GPU.
    let gpu: Bool

    private enum CodingKeys: String, CodingKey, CaseIterable { case plugins, gpu }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        plugins = try table.decodeIfPresent(String.self, forKey: .plugins)
        gpu = try table.decodeIfPresent(Bool.self, forKey: .gpu) ?? false
    }
}

/// One step: what the oracle runs (`args` and `fails`, which a phone does not read) and the calls the
/// plug-in makes to do the same.
struct Step: Decodable, Sendable {
    /// Lines `mcp` answers.
    let stdin: [String]
    let host: Host?
    let calls: [Call]

    private enum CodingKeys: String, CodingKey, CaseIterable { case args, stdin, fails, host, call }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        stdin = try table.decodeIfPresent([String].self, forKey: .stdin) ?? []
        host = try table.decodeIfPresent(Host.self, forKey: .host)
        calls = try table.decodeIfPresent([Call].self, forKey: .call) ?? []
    }
}

/// How a call that reports progress is stopped: the listener asks to stop once this fraction of the frames
/// is done, and without one the call runs to its end.
struct Stop: Decodable, Sendable {
    /// The fraction of the frames at which the listener stops the call, if it does.
    let cancelAt: Double?

    private enum CodingKeys: String, CodingKey, CaseIterable { case cancelAt = "cancel_at" }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        cancelAt = try table.decodeIfPresent(Double.self, forKey: .cancelAt)
    }

    /// Whether the listener stops the call at `done` of `total` frames. A call that has not said how many
    /// frames it has is not stopped, since half of nothing is no place to stop at.
    func stops(done: UInt64, total: UInt64) -> Bool {
        guard let cancelAt, total > 0 else { return false }
        return Double(done) >= cancelAt * Double(total)
    }
}

/// A frame's format: PNG as the engine encodes it, or straight 8-bit RGBA written out as a PNG.
enum FrameFormat: String, Decodable, Sendable {
    case png
    case rgba8
}

/// One call on the plug-in's app.
enum Call: Decodable, Sendable {
    /// A request of the engine's method table, with the files whose text it takes as parameters, and the
    /// way its progress is heard when it reports any.
    case request(method: String, params: JSONValue, files: [String: String], progress: Stop?)
    /// A frame, written to `out`.
    case frame(params: JSONValue, format: FrameFormat, out: String)
    /// Every line of the step's input, each reply a document of the output.
    case mcp

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case kind, method, params, files, progress, format, out
    }

    private enum Kind: String, Decodable { case request, frame, mcp }

    init(from decoder: any Decoder) throws {
        let kind = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self)).decode(Kind.self, forKey: .kind)
        let known: Set<String>
        switch kind {
        case .request: known = ["kind", "method", "params", "files", "progress"]
        case .frame: known = ["kind", "params", "format", "out"]
        case .mcp: known = ["kind"]
        }
        let table = try decoder.table(CodingKeys.self, allowing: known)
        let params = try table.decodeIfPresent(JSONValue.self, forKey: .params) ?? [:]
        switch kind {
        case .request:
            self = .request(
                method: try table.decode(String.self, forKey: .method),
                params: params,
                files: try table.decodeIfPresent([String: String].self, forKey: .files) ?? [:],
                progress: try table.decodeIfPresent(Stop.self, forKey: .progress)
            )
        case .frame:
            self = .frame(
                params: params,
                format: try table.decode(FrameFormat.self, forKey: .format),
                out: try table.decode(String.self, forKey: .out)
            )
        case .mcp: self = .mcp
        }
    }
}

/// The value at a pointer (RFC 6901) in the JSON document a step printed equals `equals`, or holds `length`
/// items.
struct Expect: Decodable, Sendable {
    let step: Int
    /// Which document of the step's output, from 0: `mcp` prints one per reply.
    let output: Int
    let pointer: String
    let equals: JSONValue?
    let length: Int?

    private enum CodingKeys: String, CodingKey, CaseIterable { case step, output, pointer, equals, length }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        step = try table.decode(Int.self, forKey: .step)
        output = try table.decodeIfPresent(Int.self, forKey: .output) ?? 0
        pointer = try table.decode(String.self, forKey: .pointer)
        equals = try table.decodeIfPresent(JSONValue.self, forKey: .equals)
        length = try table.decodeIfPresent(Int.self, forKey: .length)
    }
}

/// The two sides' output of a step compared: the oracle's, so a phone has no side to compare with.
struct Same: Decodable, Sendable {
    let step: Int

    private enum CodingKeys: String, CodingKey, CaseIterable { case kind, step, omit, entries, within }

    private enum Kind: String, Decodable { case output, timings }

    init(from decoder: any Decoder) throws {
        let kind = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self)).decode(Kind.self, forKey: .kind)
        let table = try decoder.table(
            CodingKeys.self,
            allowing: kind == .output ? names(CodingKeys.self) : ["kind", "step"]
        )
        step = try table.decode(Int.self, forKey: .step)
    }
}

/// A comparison of two files the steps wrote: `reference` the oracle's, `actual` the plug-in's.
struct Compare: Decodable, Sendable {
    let actual: String
    /// The two PNGs' pixels held to a tolerance, the compositor's rule for a render on the GPU against the
    /// CPU's, or nil for any other kind of comparison.
    let pixels: Tolerance?

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case kind, actual, reference
        case actualFrame = "actual_frame"
        case referenceFrame = "reference_frame"
        case exact, tolerance
        case allowedFraction = "allowed_fraction"
        case cap
    }

    private enum Kind: String, Decodable { case identical, frames, pixels, json }

    init(from decoder: any Decoder) throws {
        let kind = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self)).decode(Kind.self, forKey: .kind)
        let known: Set<String>
        switch kind {
        case .identical, .json: known = ["kind", "actual", "reference"]
        case .frames: known = ["kind", "actual", "reference", "actual_frame", "reference_frame", "exact"]
        case .pixels: known = ["kind", "actual", "reference", "tolerance", "allowed_fraction", "cap"]
        }
        let table = try decoder.table(CodingKeys.self, allowing: known)
        actual = try table.decode(String.self, forKey: .actual)
        guard kind == .pixels else {
            pixels = nil
            return
        }
        let tolerance = try table.decodeIfPresent(UInt8.self, forKey: .tolerance) ?? 0
        pixels = Tolerance(
            levels: tolerance,
            allowedFraction: try table.decodeIfPresent(Double.self, forKey: .allowedFraction) ?? 0,
            cap: try table.decodeIfPresent(UInt8.self, forKey: .cap) ?? tolerance
        )
    }
}

/// How far two PNGs' straight RGBA pixels may be apart: a pixel is over when any channel is more than
/// `levels` from the reference's, no more than `allowedFraction` of the pixels may be over, and no channel
/// may be more than `cap` from it. The defaults are the corpus's, as the desktop runner reads them.
struct Tolerance: Equatable, Sendable {
    let levels: UInt8
    let allowedFraction: Double
    let cap: UInt8
}

/// The names a folder holds when the steps are done, and no others.
struct Folder: Decodable, Sendable {
    /// Relative to the scenario's folder, `.` the folder itself.
    let path: String
    let holds: [String]

    private enum CodingKeys: String, CodingKey, CaseIterable { case path, holds }

    init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        path = try table.decode(String.self, forKey: .path)
        holds = try table.decode([String].self, forKey: .holds)
    }
}
