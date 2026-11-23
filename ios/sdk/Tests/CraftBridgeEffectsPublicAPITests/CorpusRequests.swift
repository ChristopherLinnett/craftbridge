// The conformance corpus's requests as these tests take them. The corpus is read by its one reader, the
// model of `CraftBridgeEffectsConformance` the conformance suite reads it by; what is here is what holding a
// door to its typed call needs on top of it: which of a scenario's calls go through a door, the JSON text a
// binding to another language carries a call's parameters and its reply as, and what the corpus's own rules
// say two makings of one method's request may differ in.

import CraftBridge
import CraftBridgeEffectsConformance
import Foundation

extension JSONValue {
    /// The value an object holds under `key`, or nil for a missing key or a value that is no object.
    subscript(key: String) -> JSONValue? {
        guard case .object(let fields) = self else { return nil }
        return fields[key]
    }

    /// The fields of an object, or none for any other value.
    var fields: [String: JSONValue] {
        guard case .object(let fields) = self else { return [:] }
        return fields
    }

    /// The value as the JSON text a binding hands on, through `JSONEncoder`: `JSONValue` is `Codable`, which
    /// is all another package has to write one with.
    func writtenAsJSON() throws -> String {
        String(decoding: try JSONEncoder().encode(self), as: UTF8.self)
    }

    /// The value JSON text holds, through `JSONDecoder`: how another package reads the parameters it is handed.
    init(readingJSON text: String) throws {
        self = try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }

    /// The value with whatever `pointer` (RFC 6901) names taken out: a key of an object removed, an item of
    /// an array made null, so the items after it keep their places. A pointer that names nothing changes
    /// nothing.
    func omitting(_ pointer: String) -> JSONValue {
        guard let tokens = Self.pointerTokens(pointer) else { return self }
        return omitting(tokens[...])
    }

    private func omitting(_ tokens: ArraySlice<String>) -> JSONValue {
        guard let token = tokens.first else { return self }
        let rest = tokens.dropFirst()
        switch self {
        case .object(var fields):
            if rest.isEmpty {
                fields[token] = nil
            } else if let inner = fields[token] {
                fields[token] = inner.omitting(rest)
            }
            return .object(fields)
        case .array(var items):
            guard let at = Int(token), items.indices.contains(at) else { return self }
            items[at] = rest.isEmpty ? .null : items[at].omitting(rest)
            return .array(items)
        default:
            return self
        }
    }

    /// The value with every number made zero: what is left says what was measured, and not how long it took.
    var unmeasured: JSONValue {
        switch self {
        case .number: .number(0)
        case .array(let items): .array(items.map(\.unmeasured))
        case .object(let fields): .object(fields.mapValues(\.unmeasured))
        default: self
        }
    }
}

extension Call {
    /// What ``Measured`` names the call's answer by, or nil for a line of MCP, which goes through no door:
    /// a request's method, bar a method that reports progress, which answers the render's own report
    /// whichever method it is, so what a rule of the corpus says of one such method's answer it says of
    /// every such method's.
    var measuredAs: String? {
        switch self {
        case .request(let method, _, _, nil): method
        case .request: "a render"
        case .frame: "a frame"
        case .mcp: nil
        }
    }
}

extension Step {
    /// The step's calls that go through one of the three doors, in order: every call but a line of MCP,
    /// which `mcp(_:)` takes.
    var doorCalls: [Call] { calls.filter { $0.measuredAs != nil } }

    /// Whether the step can be made only once in a process. A step whose app names a plug-in folder loads its
    /// plug-in into EffectCraft's registry of effects, which is the process's and not the engine's, so the
    /// same load on a second engine of the process is refused: the effect's id is already registered.
    var isMadeOncePerProcess: Bool { host?.plugins != nil }
}

/// What the corpus's own rules say two makings of one method's request may differ in, so that what is
/// compared is what the engine answered and never how long this machine took over it.
struct Measured: Sendable {
    /// The pointers of the values an answer measures rather than answers, such as a render's seconds, by
    /// what the answer is named by (`Call.measuredAs`).
    private var omitted: [String: Set<String>] = [:]
    /// The answers that are measurements throughout, by the same names: only what was measured is compared.
    private var throughout: Set<String> = []

    /// What the rules of `scenarios` say: a rule is about what its step prints, which is the answer to the
    /// step's last call.
    init(by scenarios: [Scenario]) {
        for scenario in scenarios {
            for rule in scenario.sames where scenario.steps.indices.contains(rule.step) {
                guard let call = scenario.steps[rule.step].calls.last?.measuredAs else { continue }
                switch rule.kind {
                case .timings: throughout.insert(call)
                case .output: omitted[call, default: []].formUnion(rule.omit)
                }
            }
        }
    }

    /// `answer`, the answer of a call named `call` here, with what the corpus says it measures taken out.
    func comparable(_ answer: JSONValue, of call: String) -> JSONValue {
        if throughout.contains(call) { return answer.unmeasured }
        return (omitted[call] ?? []).reduce(answer) { $0.omitting($1) }
    }
}

/// The corpus as these tests take it: the scenarios the plug-in runs that make a call through a door, in
/// the order the file has them, and what its rules say two makings of one request may differ in.
struct DoorCorpus: Sendable {
    let scenarios: [Scenario]
    let measured: Measured

    /// The corpus, or none of it when the file does not read, which ``GenericDoorTests`` reports.
    static let read = Result { try DoorCorpus() }

    /// The scenarios the parameterised test runs: none when the corpus does not read.
    static var scenarios: [Scenario] { (try? read.get())?.scenarios ?? [] }

    private init() throws {
        scenarios = try Corpus(file: Corpus.file).scenarios.filter { scenario in
            scenario.sides.runsPlugin && scenario.steps.contains { !$0.doorCalls.isEmpty }
        }
        measured = Measured(by: scenarios)
    }
}
