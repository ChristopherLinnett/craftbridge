// Runs the plug-in's side of a corpus scenario on a phone: each step's calls made on an engine the SDK
// starts over the scenario's own folder, and what the corpus says of the result checked. The oracle, the
// EffectCraft command line, is a program of the desktop, so what the corpus holds the two sides to each
// other for (`same`, and the comparisons of files) is the desktop runner's; here a comparison's file must
// be there and hold something. The one comparison a phone can make on its own is the compositor's GPU
// tolerance: the oracle renders on the CPU, so a scenario that asks for the GPU is made a second time on
// the CPU and its files are held to that. Every other check, the expectations and what the folders hold,
// is the corpus's own and is made as the desktop makes it.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation

/// Where the scenario's folder goes in a text of the corpus: the engine takes absolute paths, and the
/// folder is not known until the scenario runs.
private let container = "<container>"

/// A failure of the runner, not of the plug-in: the scenario cannot be judged.
struct RunnerFailure: Error, Equatable, CustomStringConvertible {
    let description: String
}

/// What one scenario came to: the failures it found, every one, and how much it ran and checked.
struct Outcome: Sendable {
    var failures: [String] = []
    var steps = 0
    /// The expectations and folder rules made, the files a comparison names that were there, and the files
    /// held to the CPU's render.
    var checks = 0
    /// The comparisons with the oracle's output or files, which the desktop runner makes.
    var withheld = 0
    /// Where each step that asked for the GPU rendered, as the engine's `info` says, and then where each
    /// rendered when the scenario was made on the CPU.
    var rendersOn: [JSONValue] = []
}

/// The kind of an error, as the plug-in's callers tell failures apart. Exhaustive, so a new kind is named
/// here before the tests build.
private func kind(of error: CraftError) -> String {
    switch error {
    case .InvalidParams: "InvalidParams"
    case .CommandFailed: "CommandFailed"
    case .NotFound: "NotFound"
    case .Unsupported: "Unsupported"
    case .Cancelled: "Cancelled"
    case .Io: "Io"
    case .Gpu: "Gpu"
    case .Internal: "Internal"
    }
}

/// A call that failed, as the document its step prints: a failure is an answer a scenario can check, as the
/// desktop's is.
private func document(of error: CraftError) -> JSONValue {
    ["error": .string(error.errorDescription ?? ""), "kind": .string(kind(of: error))]
}

extension JSONValue {
    /// The value at `pointer` (RFC 6901), if the document holds one.
    func value(at pointer: String) -> JSONValue? {
        if pointer.isEmpty { return self }
        guard pointer.hasPrefix("/") else { return nil }
        var found = self
        for token in pointer.dropFirst().split(separator: "/", omittingEmptySubsequences: false) {
            let key = token.replacingOccurrences(of: "~1", with: "/").replacingOccurrences(of: "~0", with: "~")
            switch found {
            case .object(let fields):
                guard let next = fields[key] else { return nil }
                found = next
            case .array(let items):
                guard let at = Int(key), items.indices.contains(at) else { return nil }
                found = items[at]
            default: return nil
            }
        }
        return found
    }

    /// This value with `text` written wherever a string holds `target`.
    fileprivate func replacing(_ target: String, with text: String) -> JSONValue {
        switch self {
        case .string(let value): .string(value.replacingOccurrences(of: target, with: text))
        case .array(let items): .array(items.map { $0.replacing(target, with: text) })
        case .object(let fields): .object(fields.mapValues { $0.replacing(target, with: text) })
        default: self
        }
    }

    /// The value as JSON text, which is how a failure names it.
    var shown: String { (try? jsonText()) ?? "a value JSON cannot hold" }

    /// How many items an array or an object holds, or nil for any other value.
    fileprivate var itemCount: Int? {
        switch self {
        case .array(let items): items.count
        case .object(let fields): fields.count
        default: nil
        }
    }
}

/// The folders of an app whose container is `container`, and whose bundle is `bundle`, the app's read-only
/// root: the working folders sit inside the container, each hidden from a listing of it by a leading dot.
func engineFolders(container: URL, bundle: URL?) -> Folders {
    func path(_ url: URL) -> String { url.path(percentEncoded: false) }
    let hidden = { (name: String) in path(container.appending(path: ".\(name)", directoryHint: .isDirectory)) }
    return Folders(
        container: path(container),
        bundle: bundle.map(path),
        config: hidden("config"),
        cache: hidden("cache"),
        models: hidden("models"),
        staged: hidden("staged"),
        plugins: nil,
        fonts: []
    )
}

/// The plug-in side of the corpus, run over scenario folders made in `root`.
struct ScenarioRunner: Sendable {
    /// The folder each scenario's own folder is made in, which the caller removes when it is done with it:
    /// the engine of a scenario closes on its own thread a moment after the scenario ends.
    let root: URL
    /// The corpus file's own folder, which the fixtures are read from.
    let corpusFolder: URL

    /// A runner whose scenarios' folders are made in `root`, and whose fixtures are the corpus's own.
    init(root: URL) {
        self.root = root
        corpusFolder = Corpus.file.deletingLastPathComponent()
    }

    /// Runs every step of `scenario` in a folder of its own in `root`, removed when it ends, and checks what
    /// the corpus says of the outputs and the folder. Every failure is found, not only the first: a step that
    /// cannot run ends the scenario, since what it should have written is not there to check. A scenario held
    /// to the CPU's render (``heldToTheCPU(_:)``) is made a second time, on the CPU, in a folder of its own.
    func run(_ scenario: Scenario) async -> Outcome {
        var outcome = Outcome()
        let folder = root.appending(path: scenario.id)
        let onTheCPU = Self.heldToTheCPU(scenario) ? root.appending(path: "\(scenario.id) on the CPU") : nil
        defer {
            for made in [folder, onTheCPU].compactMap({ $0 }) { try? FileManager.default.removeItem(at: made) }
        }
        do {
            let (outputs, failed, rendersOn) = try await steps(of: scenario, in: folder)
            outcome.failures += failed
            outcome.rendersOn = rendersOn
            if let onTheCPU {
                let reference = try await steps(of: scenario, in: onTheCPU, gpu: false)
                outcome.failures += reference.failures.map { "on the CPU: \($0)" }
                outcome.rendersOn += reference.rendersOn
            }
            outcome.steps = outputs.count
            outcome.failures += scenario.expects.compactMap { Self.failure(of: $0, in: outputs) }
            outcome.failures += scenario.folders.compactMap { Self.failure(of: $0, in: folder) }
            outcome.failures += scenario.compares.flatMap { Self.failures(of: $0, in: folder, onTheCPU: onTheCPU) }
            let madeHere = onTheCPU == nil ? 0 : scenario.compares.filter { $0.pixels != nil }.count
            outcome.checks = scenario.expects.count + scenario.folders.count + scenario.compares.count + madeHere
            outcome.withheld = scenario.sames.count + scenario.compares.count - madeHere
        } catch {
            outcome.failures.append("\(error)")
        }
        return outcome
    }

    /// Whether a phone holds the scenario's files to the CPU's render: its steps ask for the GPU, and a
    /// comparison holds a file to the compositor's tolerance for a render on the GPU against the CPU's,
    /// which is how the oracle renders.
    static func heldToTheCPU(_ scenario: Scenario) -> Bool {
        scenario.steps.contains { $0.host?.gpu == true } && scenario.compares.contains { $0.pixels != nil }
    }

    /// Every step of `scenario` made in `folder`, which starts with the scenario's fixtures, each step on the
    /// GPU its host asks for unless `gpu` is false: what each step printed, the failures of the plug-in, and
    /// where each step that asked for the GPU rendered. Without the oracle to say a call should fail, only the
    /// scenario's own reading of its failure (`/kind`) makes one an answer; any other is the plug-in failing.
    private func steps(of scenario: Scenario, in folder: URL, gpu: Bool = true) async throws
        -> (outputs: [[JSONValue]], failures: [String], rendersOn: [JSONValue])
    {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for fixture in scenario.fixtures { try place(fixture, in: folder) }
        var (outputs, failures, rendersOn): ([[JSONValue]], [String], [JSONValue]) = ([], [], [])
        for (index, step) in scenario.steps.enumerated() {
            do {
                let printed = try await run(step, in: folder, gpu: gpu && step.host?.gpu == true)
                outputs.append(printed.documents)
                rendersOn += printed.rendersOn.map { [$0] } ?? []
                if let error = printed.failed,
                    !scenario.expects.contains(where: { $0.step == index && $0.pointer == "/kind" })
                {
                    failures.append("step \(index): the plug-in failed: \(error.errorDescription ?? "")")
                }
            } catch {
                throw RunnerFailure(description: "step \(index): \(error)")
            }
        }
        return (outputs, failures, rendersOn)
    }

    /// A fixture copied from the corpus's folder into the scenario's.
    private func place(_ fixture: Fixture, in folder: URL) throws {
        let (from, to) = (corpusFolder.appending(path: fixture.from), folder.appending(path: fixture.to))
        try FileManager.default.createDirectory(at: to.deletingLastPathComponent(), withIntermediateDirectories: true)
        if fixture.namesContainer {
            let text = try String(contentsOf: from, encoding: .utf8)
            try text.replacingOccurrences(of: container, with: folder.path(percentEncoded: false))
                .write(to: to, atomically: true, encoding: .utf8)
        } else {
            try FileManager.default.copyItem(at: from, to: to)
        }
    }

    /// One step: an engine started afresh over the scenario's folder, on the GPU when `gpu` says, the step's
    /// calls made on it in order, and the documents of the last call's answer, or of the first failure, which
    /// ends the step, with where the engine renders when the step's host asks for the GPU. The bundle is the
    /// scenario's folder too, since the plug-in folder a scenario names sits in it and iOS loads plug-ins only
    /// from the bundle.
    private func run(_ step: Step, in folder: URL, gpu: Bool) async throws
        -> (documents: [JSONValue], failed: CraftError?, rendersOn: JSONValue?)
    {
        let engine: EffectsEngine
        do {
            let plain = engineFolders(container: folder, bundle: folder)
            engine = try await EffectsEngine.start(
                folders: step.host?.plugins.map { plain.allowingPlugins(in: $0) } ?? plain,
                gpu: gpu ? .preferred : .cpuOnly
            )
        } catch let error as CraftError {
            return ([document(of: error)], error, nil)
        }
        var rendersOn: JSONValue?
        if gpu || step.host?.gpu == true {
            let info = try await engine.request("info", [:])
            if let elsewhere = Self.failure(of: info, onTheGPU: gpu) { throw elsewhere }
            rendersOn = info.value(at: "/gpu") ?? .null
        }
        var printed: [JSONValue] = []
        for call in step.calls {
            do {
                printed = try await make(call, on: engine, in: folder, input: step.stdin)
            } catch let error as CraftError {
                return ([document(of: error)], error, rendersOn)
            }
        }
        return (printed, nil, rendersOn)
    }

    /// Why a step cannot be judged, given the engine's `info`, when it does not render where `onTheGPU` says:
    /// a step that asked for the GPU on a machine with none would run on the CPU and pass, and a CPU's render
    /// made on the GPU would hold the GPU's render to itself. The engine renders on the GPU when its session
    /// names the adapter it holds (`/project/renderer/adapter`, EffectCraft's own record). `/gpu` names the
    /// adapter or says in words why there is none, a start that found no adapter included, so it alone cannot
    /// tell the two apart.
    static func failure(of info: JSONValue, onTheGPU gpu: Bool) -> RunnerFailure? {
        let adapter = info.value(at: "/project/renderer/adapter") ?? .null
        switch (gpu, adapter) {
        case (true, .string), (false, .null): return nil
        case (true, _):
            return RunnerFailure(
                description: "the step needs the GPU, and the engine renders on the CPU: "
                    + (info.value(at: "/gpu")?.shown ?? "it says nothing of why"))
        case (false, _):
            return RunnerFailure(description: "the step's render on the CPU is on the GPU, \(adapter.shown)")
        }
    }

    /// One call's answer as the documents its step prints.
    private func make(_ call: Call, on engine: EffectsEngine, in folder: URL, input: [String]) async throws
        -> [JSONValue]
    {
        let at = folder.path(percentEncoded: false)
        switch call {
        case .request(let method, let params, let files, let stop):
            var sent = params.replacing(container, with: at)
            for (name, path) in files {
                guard case .object(var fields) = sent else {
                    throw CraftError.InvalidParams(detail: "`\(method)`: parameters that take a file must be an object")
                }
                do {
                    fields[name] = .string(try String(contentsOf: folder.appending(path: path), encoding: .utf8))
                } catch {
                    throw CraftError.Io(detail: "cannot read \(path): \(error.localizedDescription)")
                }
                sent = .object(fields)
            }
            guard let stop else { return [try await engine.request(method, sent)] }
            return [try await Listening(stop: stop).run(method, sent, on: engine)]
        case .frame(let params, let format, let out):
            let frame = try await engine.frame(
                params: params.replacing(container, with: at),
                format: format == .png ? .png : .rgba8Straight
            )
            let bytes =
                format == .png
                ? frame.data : try png(straight: frame.data, width: Int(frame.width), height: Int(frame.height))
            do {
                try bytes.write(to: folder.appending(path: out))
            } catch {
                throw CraftError.Io(detail: "cannot write \(out): \(error.localizedDescription)")
            }
            return [["width": .number(Double(frame.width)), "height": .number(Double(frame.height))]]
        case .mcp:
            var replies: [JSONValue] = []
            for line in input {
                if let reply = try await engine.mcp(line.replacingOccurrences(of: container, with: at)) {
                    replies.append(try JSONValue(jsonText: reply))
                }
            }
            return replies
        }
    }

    /// The failure of one expectation, if it does not hold.
    static func failure(of expect: Expect, in outputs: [[JSONValue]]) -> String? {
        let at = "step \(expect.step) \(expect.pointer)"
        guard outputs.indices.contains(expect.step) else { return "\(at): the step did not run" }
        let documents = outputs[expect.step]
        guard documents.indices.contains(expect.output) else {
            return "\(at): the step printed \(documents.count) documents, none at \(expect.output)"
        }
        guard let found = documents[expect.output].value(at: expect.pointer) else {
            return "\(at): the output has no such value"
        }
        if let length = expect.length {
            guard let held = found.itemCount else { return "\(at): found \(found.shown), which holds no items" }
            return held == length ? nil : "\(at): holds \(held) items, expected \(length)"
        }
        return found == expect.equals ? nil : "\(at): found \(found.shown), expected \(expect.equals?.shown ?? "none")"
    }

    /// The failure of one folder rule, if the folder does not hold exactly the names it says: the entries
    /// not starting with a dot, which are the engine's own working files.
    static func failure(of rule: Folder, in folder: URL) -> String? {
        let at = folder.appending(path: rule.path)
        guard let held = try? FileManager.default.contentsOfDirectory(atPath: at.path) else {
            return "\(rule.path): cannot read the folder"
        }
        let names = held.filter { !$0.hasPrefix(".") }.sorted()
        return names == rule.holds.sorted() ? nil : "\(rule.path) holds \(names), expected \(rule.holds.sorted())"
    }

    /// The failures of one comparison of files, as far as a phone makes it: the plug-in's file is there and
    /// holds something, and, when `onTheCPU` holds the scenario made on the CPU, a file held to the GPU's
    /// tolerance is within it of the CPU's. What else it is held against is the oracle's, which only the
    /// desktop has.
    static func failures(of compare: Compare, in folder: URL, onTheCPU: URL? = nil) -> [String] {
        let file = folder.appending(path: compare.actual)
        var isFolder: ObjCBool = false
        guard FileManager.default.fileExists(atPath: file.path, isDirectory: &isFolder) else {
            return ["\(compare.actual): the step wrote no such file"]
        }
        let held =
            isFolder.boolValue
            ? (try? FileManager.default.contentsOfDirectory(atPath: file.path))?.count
            : (try? FileManager.default.attributesOfItem(atPath: file.path)[.size] as? Int)
        guard (held ?? 0) > 0 else { return ["\(compare.actual): is empty"] }
        guard let tolerance = compare.pixels, let onTheCPU else { return [] }
        return pixels(file, against: onTheCPU.appending(path: compare.actual), within: tolerance).map {
            "\(compare.actual) against the CPU's: \($0)"
        }
    }
}
