// The text form of the generic door: the parameters reach the engine as the caller wrote them, and the
// reply comes back as the engine wrote it. The engine handle is the shared stand-in, which answers fixed
// text, so that what is held is the SDK's own handling of the text. A real engine's replies are held from
// outside the package, in the public API tests, which cannot hold a whole number above 2^53: the real
// engine keeps every number as a double and answers none.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing

/// A reply whose keys are in no order a `Dictionary` keeps, holding a whole number written `2.0`, whole
/// numbers a `Double` cannot hold, a spacing that writing it again would drop, and a letter spelt both ways,
/// with a combining accent and as one character, which a `String` compares equal: a door that normalised
/// to either form would change one of them.
private let engineText =
    #"{"zeta": 1, "two": 2.0, "alpha": 9007199254740993, "mid": {"z": true, "a": null}, "#
    + #""id": 18446744073709551615, "name": "Cafe\#u{301}", "nfc": "Caf\#u{e9}"}"#

/// Parameters that are valid JSON but that no encoder would write the same: the spacing, a repeated key, a
/// number past `Double`, and a letter spelt both ways (see ``engineText``).
private let oddParams = #"{ "b":1,  "a":9007199254740993, "b":2, "s":"e\#u{301}", "p":"\#u{e9}" }"#

/// The bytes of `text`: the door keeps a text's spelling, which `==` on a `String` does not compare.
private func bytes(_ text: String) -> [UInt8] { Array(text.utf8) }

private let refusal = CraftError.InvalidParams(detail: "the engine's refusal")

/// What a `JSONValue` call throws for a reply of the engine's that is not JSON.
private let unreadable = CraftError.Internal(detail: "the engine answered with text that is not JSON")

/// An engine over the stand-in handle, which answers `text` to every request and call with progress.
private func engine(answering text: String = engineText, over requests: Requests) -> EffectsEngine {
    let handle = FakeEngine(recording: requests)
    handle.answer = { _, _ in text }
    return EffectsEngine(engine: handle)
}

@Suite struct TextDoorTests {
    @Test func aRequestHandsItsParametersOverUnchangedAndAnswersTheEnginesTextUnchanged() async throws {
        let requests = Requests()
        let reply = try await engine(over: requests).requestJSON("exec", params: oddParams)
        #expect(bytes(reply) == bytes(engineText))
        #expect(await requests.texts.map(bytes) == [bytes(oddParams)])
        #expect(await requests.method() == "exec")
    }

    @Test func aFrameHandsItsParametersOverUnchanged() async throws {
        let requests = Requests()
        _ = try await engine(over: requests).frameJSON(params: oddParams, format: .rgba8Straight)
        #expect(await requests.texts.map(bytes) == [bytes(oddParams)])
        #expect(await requests.method() == "frame")
    }

    @Test func aJobHandsItsParametersOverUnchangedAndAnswersTheEnginesTextAnyNumberOfTimes() async throws {
        let requests = Requests()
        let job = try await engine(over: requests).jobJSON("export", params: oddParams)
        let value = try JSONValue(jsonText: engineText)
        #expect(bytes(try await job.resultJSON) == bytes(engineText))
        #expect(bytes(try await job.resultJSON) == bytes(engineText), "the text can be awaited again")
        #expect(try await job.result == value, "the value reads the same text")
        #expect(try await job.result == value, "and again")
        #expect(await requests.texts.map(bytes) == [bytes(oddParams)])
    }

    /// Cancelling a task that awaits the text cancels the job, as cancelling one that awaits the value does.
    /// The engine's answer is held back until the task is cancelled, so the job cannot have ended first.
    @Test func cancellingTheTaskAwaitingTheTextCancelsTheJob() async throws {
        let handle = FakeEngine()
        let (held, release) = AsyncStream<Void>.makeStream()
        handle.answer = { _, _ in
            for await _ in held {}
            return engineText
        }
        let job = try await EffectsEngine(engine: handle).jobJSON("export", params: "{}")
        let awaiting = Task { try await job.resultJSON }
        awaiting.cancel()
        release.finish()
        #expect(bytes(try await awaiting.value) == bytes(engineText), "the stand-in answers whatever the token says")
        #expect(job.isCancelled)
    }

    @Test func textThatIsNotJSONIsHandedOverUnparsedForTheEngineToRefuse() async throws {
        let requests = Requests()
        let handle = FakeEngine(recording: requests)
        handle.answer = { _, _ in throw refusal }
        let engine = EffectsEngine(engine: handle)
        await #expect(throws: refusal) { try await engine.requestJSON("info", params: "not json") }
        let job = try await engine.jobJSON("export", params: "not json")
        for _ in 0..<2 {
            await #expect(throws: refusal) { try await job.resultJSON }
            await #expect(throws: refusal) { try await job.result }
        }
        #expect(await requests.texts == ["not json", "not json"], "the engine was handed the text as it was")
    }

    @Test func aJobWhoseTextIsNotJSONAnswersItAsTextAndFailsAsAValueWhereTheValueIsRead() async throws {
        let job = try await engine(answering: "{", over: Requests()).jobJSON("export", params: "{}")
        #expect(try await job.resultJSON == "{")
        await withTaskGroup(of: Void.self) { readers in
            for _ in 0..<8 {
                readers.addTask { await #expect(throws: unreadable) { try await job.result } }
            }
        }
        #expect(try await job.resultJSON == "{", "a value that failed leaves the text as it was")
    }

    @Test func aReplyThatIsNotJSONIsAnsweredAsTextAndRefusedAsAValue() async throws {
        let engine = engine(answering: "{", over: Requests())
        #expect(try await engine.requestJSON("info", params: "{}") == "{")
        await #expect(throws: unreadable) { try await engine.request("info", [:]) }
    }

    @Test func theValueCallsWriteTheirParametersAsTheTextCallsAreHandedThem() async throws {
        let requests = Requests()
        let engine = engine(over: requests)
        let params: JSONValue = ["a": [1, "x", nil], "b": true]
        _ = try await engine.request("exec", params)
        _ = try await engine.frame(params: params, format: .png)
        _ = try await engine.job("export", params).result
        // A dictionary writes its keys in no fixed order, so what each call handed over is read as a value.
        let handed = try await requests.texts.map { try JSONValue(jsonText: $0) }
        #expect(handed == [params, params, params], "each call reached the handle as the text of its value")
        #expect(try await engine.request("exec", params) == (try JSONValue(jsonText: engineText)))
    }

    @Test func parametersJSONCannotCarryAreRefusedBeforeAnyCallReachesTheEngine() async throws {
        let requests = Requests()
        let engine = engine(over: requests)
        let refused = CraftError.InvalidParams(
            detail: "the parameters hold NaN or an infinity, which JSON has no number for")
        let nan: JSONValue = ["time": .number(.nan)]
        await #expect(throws: refused) { try await engine.request("info", nan) }
        await #expect(throws: refused) { _ = try await engine.frame(params: nan, format: .png) }
        await #expect(throws: refused) { _ = try await engine.job("export", nan) }
        #expect(await requests.texts.isEmpty)
    }
}
