// The generic door from outside the package: `request(_:_:)`, `frame(params:format:)` and
// `job(_:_:progress:)` each answer what the typed call answers for the same request. The requests are the
// conformance corpus's. Every scenario's steps are made twice over one folder, first with every call through
// the door and then with each call through the typed call that spells it, and the two answers of each such
// call are held to each other; a step that can be made only once in a process is made through the door
// alone. What the corpus expects of an answer is the conformance suite's to check; here the two ways to ask
// are held to say the same.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeEffectsConformance
import Foundation
import Testing

/// Which of the three doors a call of the corpus goes through.
enum Door: String, CaseIterable, Sendable {
    case request = "request(_:_:)"
    case frame = "frame(params:format:)"
    case job = "job(_:_:progress:)"
}

/// What a frame call asks for in the tally of the calls made, where a request has its method's name.
private let frameMethod = "frame"

/// How a scenario's calls reach the engine: all through the generic door, or each through the typed call
/// that spells it, where there is one.
enum Route: Sendable {
    case door
    case typed
}

/// What one call answered. A refusal is an answer too: the two ways to ask must refuse alike.
enum Answer: Equatable, Sendable, CustomStringConvertible {
    case reply(JSONValue)
    case picture(width: UInt32, height: UInt32, format: String, bytes: Data)
    case refused(CraftError)

    var description: String {
        switch self {
        case .reply(let reply): (try? reply.writtenAsJSON()) ?? "a reply JSON cannot hold"
        case .picture(let width, let height, let format, let bytes):
            "a \(width) by \(height) \(format) frame of \(bytes.count) bytes, hashing to \(bytes.hashValue)"
        case .refused(let error): "refused: \(error)"
        }
    }
}

/// One call as it was made: the door it goes through, the method it asks, what the corpus's rules name its
/// answer by, whether a typed call made it, and what it answered.
struct Made: Sendable {
    let door: Door
    let method: String
    let measuredAs: String
    let typed: Bool
    let answer: Answer

    /// The answer with what the corpus says it measures taken out of a reply.
    func comparable(by measured: Measured) -> Answer {
        guard case .reply(let reply) = answer else { return answer }
        return .reply(measured.comparable(reply, of: measuredAs))
    }
}

/// A scenario's calls made over `container`, each step on an engine started afresh there, as the corpus
/// has every step start: what one step leaves for the next is in the folder's files.
private struct Replay {
    let container: URL
    let route: Route

    /// What every call of every step answered, or nil for a step left out: one that can be made only once
    /// in a process, which the making through the door has made by the time the typed calls come to it.
    func answers(of scenario: Scenario) async throws -> [[Made]?] {
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        for fixture in scenario.fixtures { try fixture.place(in: container) }
        var answers: [[Made]?] = []
        for step in scenario.steps {
            let leftOut = route == .typed && step.isMadeOncePerProcess
            answers.append(leftOut ? nil : try await self.answers(of: step))
        }
        return answers
    }

    /// One step's calls, in order, up to the first the engine refuses, which ends the step. Every step
    /// renders on the CPU, whatever its host asks: where a frame renders is no part of how it was asked
    /// for, and two renders of one request are the same bytes on the CPU, which two on a GPU need not be.
    /// The engine is shut down before the step answers, so the next step's engine starts over the folder
    /// of one that has ended.
    private func answers(of step: Step) async throws -> [Made] {
        let plain = engineFolders(in: container)
        let folders = step.host?.plugins.map { plain.allowingPlugins(in: $0) } ?? plain
        let engine = try await EffectsEngine.start(folders: folders, gpu: .cpuOnly)
        var made: [Made] = []
        do {
            for call in step.doorCalls {
                guard let one = try await make(call, on: engine) else { continue }
                made.append(one)
                if case .refused = one.answer { break }
            }
        } catch {
            try? await engine.shutDown()
            throw error
        }
        try await engine.shutDown()
        return made
    }

    /// What `call` answers, the engine's refusal being an answer.
    private func answered(_ call: () async throws -> Answer) async throws -> Answer {
        do {
            return try await call()
        } catch let error as CraftError {
            return .refused(error)
        }
    }

    /// The parameters a door is handed: `params` through JSON text and back, the round the parameters of a
    /// binding's call make on their way to it.
    private func handed(_ params: JSONValue) throws -> JSONValue {
        try JSONValue(readingJSON: try params.writtenAsJSON())
    }

    /// `call` as it was made on `engine`, or nil for a call that goes through no door.
    private func make(_ call: Call, on engine: EffectsEngine) async throws -> Made? {
        guard let (door, method) = door(of: call), let measuredAs = call.measuredAs,
            let (answer, typed) = try await answer(to: call, on: engine)
        else { return nil }
        return Made(door: door, method: method, measuredAs: measuredAs, typed: typed, answer: answer)
    }

    /// What `call` answers on `engine`, and whether a typed call made it, or nil for a call that goes through
    /// no door.
    private func answer(to call: Call, on engine: EffectsEngine) async throws -> (Answer, typed: Bool)? {
        switch call {
        case .request(let method, let params, let files, nil):
            let sent = try Call.parameters(of: method, params, files: files, in: container)
            let typed = route == .typed ? typedRequest(method, sent) : nil
            let answer = try await answered {
                if let typed { return .reply(try await typed(engine)) }
                return .reply(try await engine.request(method, try handed(sent)))
            }
            return (answer, typed != nil)
        case .request(let method, let params, let files, let stop?):
            let sent = try Call.parameters(of: method, params, files: files, in: container)
            let typed = route == .typed ? typedJob(method, sent) : nil
            let hearing = Hearing(stop: stop)
            let progress: ProgressHandler = { done, total, _ in hearing.hear(done: done, total: total) }
            let answer = try await answered {
                let job =
                    if let typed {
                        try await typed(engine, progress)
                    } else {
                        try await engine.job(method, try handed(sent), progress: progress)
                    }
                hearing.take(job)
                return .reply(try await job.result)
            }
            return (answer, typed != nil)
        case .frame(let params, let format, _):
            let sent = params.replacing(containerMarker, with: container.path(percentEncoded: false))
            let typed = route == .typed ? TypedFrame(sent) : nil
            let layout: PixelFormat = format == .png ? .png : .rgba8Straight
            let answer = try await answered {
                let frame: FrameBytes =
                    if let typed {
                        try await engine.frame(
                            comp: typed.comp, at: typed.time, frame: typed.frame, maxSide: typed.maxSide,
                            scale: typed.scale, transparent: typed.transparent, format: layout)
                    } else {
                        try await engine.frame(params: try handed(sent), format: layout)
                    }
                return .picture(
                    width: frame.width, height: frame.height, format: "\(frame.format)", bytes: frame.data)
            }
            return (answer, typed != nil)
        case .mcp:
            return nil
        }
    }
}

/// The methods of the corpus's requests a typed call covers. The list stands apart from the typed calls'
/// spelling in `TypedCalls.swift` because that spelling is what it is held to: a method left out there by
/// mistake is then a request this list says a typed call must spell.
private let coveredMethods: Set<String> = [
    "exec", "run", "props", "get", "set", "replay", "timings", "export", "template.render",
]

/// The door a call goes through, and the method it asks, or nil for a line of MCP, which goes through none.
private func door(of call: Call) -> (door: Door, method: String)? {
    switch call {
    case .request(let method, _, _, nil): (.request, method)
    case .request(let method, _, _, _?): (.job, method)
    case .frame: (.frame, frameMethod)
    case .mcp: nil
    }
}

/// A folder to read a call's parameters in when only what they name matters, and not where.
private let anywhere = URL(filePath: "/", directoryHint: .isDirectory)

/// Whether a typed call spells `call`, read from the corpus alone: a parameter a file holds is a text,
/// whatever the file says.
private func isSpelt(_ call: Call) throws -> Bool {
    switch call {
    case .request(let method, let params, let files, let stop):
        let sent = try Call.parameters(of: method, params, files: files, in: anywhere) { _ in "" }
        return stop == nil ? typedRequest(method, sent) != nil : typedJob(method, sent) != nil
    case .frame(let params, _, _):
        return TypedFrame(params) != nil
    case .mcp:
        return false
    }
}

/// Why no typed call spells `call`, for the requests the typed calls do not cover, which is what the door
/// is for; or nil for a request a typed call must spell, so that one it does not is a failure of these
/// tests or of the typed call, and never a request quietly held to nothing.
private func whyTheDoorAlone(makes call: Call) -> String? {
    switch call {
    case .request(let method, _, _, _) where !coveredMethods.contains(method):
        "no typed call asks `\(method)`"
    case .request("export", let params, _, _) where params["output"] == nil:
        "the typed `export` names the file it writes, and an export of the project's own queue names none"
    case .frame(let params, _, _) where params["template"] != nil:
        "the typed `frame` has no argument for a template"
    default:
        nil
    }
}

@Suite(.serialized, .timeLimit(.minutes(10))) struct GenericDoorTests {
    /// Every door has requests of the corpus that a typed call spells, so no door is held to nothing; and
    /// the requests no typed call spells are exactly those the typed calls do not cover.
    @Test func everyDoorHasRequestsOfTheCorpusThatATypedCallSpells() throws {
        let corpus = try DoorCorpus.read.get()
        #expect(!corpus.scenarios.isEmpty, "the corpus holds scenarios that make a call through a door")
        var held: [Door: Int] = [:]
        var notHeld: [String] = []
        for scenario in corpus.scenarios {
            for step in scenario.steps {
                for call in step.doorCalls {
                    guard let (door, method) = door(of: call) else { continue }
                    let why = whyTheDoorAlone(makes: call)
                    let spelt = try isSpelt(call)
                    #expect(
                        spelt == (why == nil),
                        "\(scenario.id): a typed call \(spelt ? "spells" : "does not spell") a `\(method)` request")
                    if step.isMadeOncePerProcess {
                        notHeld.append("\(scenario.id) `\(method)`: its step can be made once in a process")
                    } else if let why {
                        notHeld.append("\(scenario.id) `\(method)`: \(why)")
                    } else {
                        held[door, default: 0] += 1
                    }
                }
            }
        }
        for door in Door.allCases {
            #expect(held[door, default: 0] > 0, "no request of the corpus holds \(door.rawValue) to a typed call")
        }
        let counts = Door.allCases.map { "\(held[$0, default: 0]) through \($0.rawValue)" }.joined(separator: ", ")
        print("the corpus holds \(counts) to a typed call, and makes through the door alone:")
        for one in notHeld { print("  \(one)") }
    }

    @Test(arguments: DoorCorpus.scenarios)
    func everyDoorAnswersWhatTheTypedCallAnswers(_ scenario: Scenario) async throws {
        let measured = try DoorCorpus.read.get().measured
        let container = scratchFolder("door")
        defer { try? FileManager.default.removeItem(at: container) }
        let door = try await Replay(container: container, route: .door).answers(of: scenario).compactMap { $0 }
        // The second making starts where the first did: in the same folder, emptied, so an answer that
        // names a path names the same one.
        try FileManager.default.removeItem(at: container)
        let typed = try await Replay(container: container, route: .typed).answers(of: scenario)

        #expect(door.count == typed.count, "\(scenario.id): the two makings took as many steps")
        var held = 0
        for (step, (asked, spelt)) in zip(door, typed).enumerated() {
            guard let spelt else { continue }
            #expect(
                asked.count == spelt.count,
                "\(scenario.id), step \(step): the two ways to ask made as many calls")
            for (through, other) in zip(asked, spelt) where other.typed {
                held += 1
                #expect(
                    through.comparable(by: measured) == other.comparable(by: measured),
                    """
                    \(scenario.id), step \(step): \(through.door.rawValue) answered `\(through.method)` with \
                    \(through.answer), and the typed call with \(other.answer)
                    """)
            }
        }
        // A reply is JSON a binding hands on: written as text and read back, it is the reply.
        for case .reply(let reply) in door.joined().map(\.answer) {
            #expect(try JSONValue(readingJSON: try reply.writtenAsJSON()) == reply)
        }
        print("\(scenario.id): \(door.joined().count) calls through the door, \(held) held to their typed calls")
    }

    /// The round a binding's call makes: its parameters arrive as JSON text, and its reply leaves as JSON
    /// text, through nothing but `Codable`.
    @Test func aCallGoesInAsJSONTextAndItsReplyComesBackAsJSONText() async throws {
        try await withEngine("text") { engine, _ in
            let made = try JSONValue(
                readingJSON: #"{"command":"comp.new","params":{"name":"Card","width":64,"height":36}}"#)
            _ = try await engine.request("exec", made)
            let asked = try JSONValue(readingJSON: #"{"command":"comp.info","params":{"comp":"Card"}}"#)
            let text = try await engine.request("exec", asked).writtenAsJSON()
            let reply = try JSONValue(readingJSON: text)
            #expect(!reply.fields.isEmpty, "the reply's text holds the composition the first call made: \(text)")
            let typed = try await engine.exec("comp.info", params: ["comp": "Card"])
            #expect(reply == typed, "the text read back is what the typed call answers")
            // A number is a `Double` on its way through, so a whole one is written without a fraction.
            #expect(try JSONValue(readingJSON: "3.0").writtenAsJSON() == "3")
        }
    }

    /// The engine answers each method at one door alone, and a binding must send it there: `request` refuses
    /// a method that reports progress, and a frame, and a job of a method that reports none is refused, each
    /// as `Unsupported` in words that name the method asked.
    @Test func everyDoorRefusesAMethodAnotherDoorAnswers() async throws {
        try await withEngine("doors") { engine, _ in
            let export = await #expect(throws: CraftError.self) { try await engine.request("export", [:]) }
            let frame = await #expect(throws: CraftError.self) { try await engine.request("frame", [:]) }
            let info = await #expect(throws: CraftError.self) { try await engine.job("info", [:]).result }
            for (refusal, method) in [(export, "export"), (frame, "frame"), (info, "info")] {
                guard case .Unsupported(let detail)? = refusal else {
                    Issue.record("a door that does not answer `\(method)` ended it in \(String(describing: refusal))")
                    continue
                }
                #expect(detail.hasPrefix("`\(method)` is not answered by "), "the refusal names its method: \(detail)")
            }
        }
    }

    /// Parameters JSON cannot carry are refused at each door before anything reaches the engine, in the
    /// words every call's parameters are refused with.
    @Test func parametersJSONCannotCarryAreRefusedAtEveryDoor() async throws {
        let refused = CraftError.InvalidParams(
            detail: "the parameters hold NaN or an infinity, which JSON has no number for")
        let unwritable: JSONValue = ["time": .number(.nan)]
        try await withEngine("refused") { engine, _ in
            await #expect(throws: refused) { try await engine.request("info", unwritable) }
            await #expect(throws: refused) { _ = try await engine.frame(params: unwritable, format: .png) }
            await #expect(throws: refused) { _ = try await engine.job("export", unwritable) }
        }
    }
}
