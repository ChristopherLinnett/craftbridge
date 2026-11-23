// The conformance corpus as Swift reads it: `conformance/effects/corpus.toml`, the one file every runner
// reads, parsed by TOMLDecoder and never copied, so a scenario is written once. The model is strict, as
// the desktop runner's is: a key or a kind the runner does not read is refused, never ignored, so a check
// the corpus gains cannot go unrun here. It is a module of its own, in no product, because the package's
// test targets each read the corpus and a test target can share no source with another.

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

/// A file of `conformance/effects`, the data every platform reads. On a phone it is the copy the device runner
/// puts inside the app (`conformance/effects` at the bundle's root), since the machine that built the tests is
/// not there; everywhere else it is found from where the compiler read this source, which is the repository on
/// the machine that builds the tests, the simulator's host.
public func conformanceFile(_ name: String) -> URL {
    let bundled = Bundle.main.bundleURL.appending(path: "conformance/effects/\(name)")
    if FileManager.default.fileExists(atPath: bundled.path(percentEncoded: false)) { return bundled }
    return URL(filePath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent()
        .appending(path: "conformance/effects/\(name)")
}

extension Corpus {
    /// The corpus file the desktop runner reads.
    public static let file = conformanceFile("corpus.toml")
}

/// Which sides run a scenario.
public enum Sides: String, Decodable, Sendable {
    /// The oracle and the plug-in both run it.
    case both
    /// The oracle alone runs it.
    case oracle
    /// The plug-in alone runs it.
    case plugin

    /// Whether the plug-in side, the one a phone runs, takes part.
    public var runsPlugin: Bool { self != .oracle }
}

/// The corpus: its scenarios, in the order the file has them.
public struct Corpus: Decodable, Sendable {
    /// The scenarios, in the order the file has them.
    public let scenarios: [Scenario]

    private enum CodingKeys: String, CodingKey, CaseIterable { case scenario }

    /// Reads the corpus's one table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        scenarios = try table.decode([Scenario].self, forKey: .scenario)
    }

    /// The corpus `text` holds, parsed as TOML.
    public init(toml text: String) throws {
        let json = try JSONValue(toml: text).jsonText()
        self = try JSONDecoder().decode(Corpus.self, from: Data(json.utf8))
    }

    /// The corpus in `file`.
    public init(file: URL) throws {
        try self.init(toml: try String(contentsOf: file, encoding: .utf8))
    }
}

extension JSONValue {
    /// The JSON value a TOML document holds, its refusals naming it as `file`. TOMLDecoder reads the document
    /// and gives each value its Swift type; a date or time, which JSON has no value for, is refused, neither the
    /// corpus nor the baseline having one.
    public init(toml text: String, of file: String = "the corpus") throws {
        self = try JSONValue(tomlValue: try [String: Any](TOMLTable(source: text)), of: file)
    }

    private init(tomlValue value: Any, of file: String) throws {
        switch value {
        case let value as Bool: self = .bool(value)
        case let value as Int64: self = .number(Double(value))
        case let value as Double: self = .number(value)
        case let value as String: self = .string(value)
        case let value as [Any]: self = .array(try value.map { try JSONValue(tomlValue: $0, of: file) })
        case let value as [String: Any]: self = .object(try value.mapValues { try JSONValue(tomlValue: $0, of: file) })
        default: throw CraftError.InvalidParams(detail: "\(file) holds a TOML value JSON has none for: \(value)")
        }
    }
}

/// One scenario: its steps, and what to check after them.
public struct Scenario: Decodable, Sendable, CustomStringConvertible {
    /// The plan's identifier (`T5`), which every failure names.
    public let id: String
    /// What the scenario shows, in a line.
    public let title: String
    /// Which sides run it.
    public let sides: Sides
    /// Files put in the folder before the first step.
    public let fixtures: [Fixture]
    /// The steps, in the order they run.
    public let steps: [Step]
    /// Values the step outputs must hold.
    public let expects: [Expect]
    /// Outputs the two sides must print alike: the oracle's, so held on the desktop.
    public let sames: [Same]
    /// Files the sides must hold alike, the oracle's against the plug-in's: held on the desktop.
    public let compares: [Compare]
    /// The names the folders must hold.
    public let folders: [Folder]

    /// The id and the title, which a test log names the scenario's case by.
    public var description: String { "\(id): \(title)" }

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case id, title, sides, fixture, step, expect, same, compare, folder
    }

    /// Reads one scenario's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
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
public struct Fixture: Decodable, Sendable {
    /// Relative to the corpus file's folder.
    public let from: String
    /// Relative to the scenario's folder.
    public let to: String
    /// The file's text says `<container>` where the scenario's folder goes.
    public let namesContainer: Bool

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case from, to
        case namesContainer = "names_container"
    }

    /// Reads one fixture's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        from = try table.decode(String.self, forKey: .from)
        to = try table.decode(String.self, forKey: .to)
        namesContainer = try table.decodeIfPresent(Bool.self, forKey: .namesContainer) ?? false
    }
}

/// How the plug-in's app is built for one step.
public struct Host: Decodable, Sendable {
    /// The plug-in folder the app names, relative to the scenario's folder; plug-ins are off without one.
    public let plugins: String?
    /// The app is handed the GPU.
    public let gpu: Bool

    private enum CodingKeys: String, CodingKey, CaseIterable { case plugins, gpu }

    /// Reads one host's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        plugins = try table.decodeIfPresent(String.self, forKey: .plugins)
        gpu = try table.decodeIfPresent(Bool.self, forKey: .gpu) ?? false
    }
}

/// One step: what the oracle runs (`args` and `fails`, which a phone does not read) and the calls the
/// plug-in makes to do the same.
public struct Step: Decodable, Sendable {
    /// Lines `mcp` answers.
    public let stdin: [String]
    /// How the plug-in's app is built for the step, when the step says.
    public let host: Host?
    /// The calls the plug-in makes, in order.
    public let calls: [Call]

    private enum CodingKeys: String, CodingKey, CaseIterable { case args, stdin, fails, host, call }

    /// Reads one step's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        stdin = try table.decodeIfPresent([String].self, forKey: .stdin) ?? []
        host = try table.decodeIfPresent(Host.self, forKey: .host)
        calls = try table.decodeIfPresent([Call].self, forKey: .call) ?? []
    }
}

/// How a call that reports progress is stopped: the listener asks to stop once this fraction of the frames
/// is done, and without one the call runs to its end.
public struct Stop: Decodable, Sendable {
    /// The fraction of the frames at which the listener stops the call, if it does.
    public let cancelAt: Double?

    private enum CodingKeys: String, CodingKey, CaseIterable { case cancelAt = "cancel_at" }

    /// Reads one stop's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        cancelAt = try table.decodeIfPresent(Double.self, forKey: .cancelAt)
    }

    /// Whether the listener stops the call at `done` of `total` frames. A call that has not said how many
    /// frames it has is not stopped, since half of nothing is no place to stop at.
    public func stops(done: UInt64, total: UInt64) -> Bool {
        guard let cancelAt, total > 0 else { return false }
        return Double(done) >= cancelAt * Double(total)
    }
}

/// A frame's format: PNG as the engine encodes it, or straight 8-bit RGBA written out as a PNG.
public enum FrameFormat: String, Decodable, Sendable {
    /// PNG, as the engine encodes it.
    case png
    /// Rows of straight 8-bit RGBA.
    case rgba8
}

/// One call on the plug-in's app.
public enum Call: Decodable, Sendable {
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

    /// Reads one call's table, refusing a kind the runner does not make and a key its kind does not have.
    public init(from decoder: any Decoder) throws {
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
public struct Expect: Decodable, Sendable {
    /// The step whose output is read, from 0.
    public let step: Int
    /// Which document of the step's output, from 0: `mcp` prints one per reply.
    public let output: Int
    /// Where in the document the value sits.
    public let pointer: String
    /// What the value must equal, when the expectation names a value.
    public let equals: JSONValue?
    /// How many items the value must hold, when the expectation names a count.
    public let length: Int?

    private enum CodingKeys: String, CodingKey, CaseIterable { case step, output, pointer, equals, length }

    /// Reads one expectation's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        step = try table.decode(Int.self, forKey: .step)
        output = try table.decodeIfPresent(Int.self, forKey: .output) ?? 0
        pointer = try table.decode(String.self, forKey: .pointer)
        equals = try table.decodeIfPresent(JSONValue.self, forKey: .equals)
        length = try table.decodeIfPresent(Int.self, forKey: .length)
    }
}

/// The two sides' output of a step compared: the oracle's, so a phone has no side to compare with. What a
/// rule lets the two sides differ in is what a step's answer measures rather than answers, so it is also what
/// two makings of the step on one machine may differ in.
public struct Same: Decodable, Sendable {
    /// What a rule compares.
    public enum Kind: String, Decodable, Sendable {
        /// The documents the step printed, apart from what the rule leaves out.
        case output
        /// Which runs, layers, effects and playback a `timings` timed, and no measurement.
        case timings
    }

    /// What the rule compares.
    public let kind: Kind
    /// The step whose output is compared, from 0.
    public let step: Int
    /// The pointers (RFC 6901) of the values an `output` rule leaves out of the comparison.
    public let omit: [String]

    private enum CodingKeys: String, CodingKey, CaseIterable { case kind, step, omit, entries, within }

    /// Reads one rule's table, refusing a key its kind does not have.
    public init(from decoder: any Decoder) throws {
        kind = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self)).decode(Kind.self, forKey: .kind)
        let table = try decoder.table(
            CodingKeys.self,
            allowing: kind == .output ? names(CodingKeys.self) : ["kind", "step"]
        )
        step = try table.decode(Int.self, forKey: .step)
        omit = try table.decodeIfPresent([String].self, forKey: .omit) ?? []
    }
}

/// A comparison of two files the steps wrote: `reference` the oracle's, `actual` the plug-in's.
public struct Compare: Decodable, Sendable {
    /// The plug-in's file, relative to the scenario's folder.
    public let actual: String
    /// The two PNGs' pixels held to a tolerance, the compositor's rule for a render on the GPU against the
    /// CPU's, or nil for any other kind of comparison.
    public let pixels: Tolerance?

    private enum CodingKeys: String, CodingKey, CaseIterable {
        case kind, actual, reference
        case actualFrame = "actual_frame"
        case referenceFrame = "reference_frame"
        case exact, tolerance
        case allowedFraction = "allowed_fraction"
        case cap
    }

    private enum Kind: String, Decodable { case identical, frames, pixels, json }

    /// Reads one comparison's table, refusing a kind the runner does not know and a key its kind does not have.
    public init(from decoder: any Decoder) throws {
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
public struct Tolerance: Equatable, Sendable {
    /// How far a channel may be from the reference's before its pixel is over.
    public let levels: UInt8
    /// The fraction of the pixels that may be over.
    public let allowedFraction: Double
    /// How far any channel may be from the reference's at all.
    public let cap: UInt8

    /// A tolerance of `levels`, with `allowedFraction` of the pixels allowed over it and none past `cap`.
    public init(levels: UInt8, allowedFraction: Double, cap: UInt8) {
        self.levels = levels
        self.allowedFraction = allowedFraction
        self.cap = cap
    }
}

/// The names a folder holds when the steps are done, and no others.
public struct Folder: Decodable, Sendable {
    /// Relative to the scenario's folder, `.` the folder itself.
    public let path: String
    /// The names the folder holds, and no others.
    public let holds: [String]

    private enum CodingKeys: String, CodingKey, CaseIterable { case path, holds }

    /// Reads one folder rule's table, refusing a key the runner does not read.
    public init(from decoder: any Decoder) throws {
        let table = try decoder.table(CodingKeys.self, allowing: names(CodingKeys.self))
        path = try table.decode(String.self, forKey: .path)
        holds = try table.decode([String].self, forKey: .holds)
    }
}
