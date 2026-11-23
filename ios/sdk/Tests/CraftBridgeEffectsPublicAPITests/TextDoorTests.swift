// The generic door's JSON text forms from outside the package, on a real engine: what a binding to another
// language is handed is the reply exactly as the engine wrote it, and text that is not JSON is the
// engine's own refusal. What the text keeps that a `JSONValue` does not is held against the `JSONValue`
// path of the same request. A whole number above 2^53, and a job's `2.0`, are held in the package's own
// tests, over the boundary's stand-in: the real engine keeps every number as a double and answers no such
// whole number, and the one number of a real export's report is a measured time.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing

/// The keys of the reply `get` answers for a layer's position, in the order the engine writes them. The
/// order is no dictionary's: a `Dictionary` keeps the order of a per-process seed.
private let positionKeys = [
    "animated", "expression", "keys", "layer", "match", "name", "time", "type", "uid", "value",
]

/// The request that makes a comp and one layer to ask about.
private let makeCard =
    #"{"steps":[{"command":"comp.new","params":{"name":"Card","width":64,"height":36}},"#
    + #"{"command":"layer.newSolid","params":{"name":"S","width":64,"height":36}}]}"#

/// The request for the position of that layer.
private let askPosition = ##"{"comp":"Card","layer":"#1","path":"transform/position"}"##

/// What the engine answers a request with that is not JSON at its first character: serde_json's own words,
/// which Kotlin's door throws as `InvalidParamsException` for the same text.
private let notJSONRefusal = CraftError.InvalidParams(detail: "expected ident at line 1 column 2")

@Suite(.serialized, .timeLimit(.minutes(5))) struct TextDoorTests {
    /// The reply's numbers come back as the engine wrote them, `32.0` with its fraction, and its keys in the
    /// engine's order, whichever process asks: a reply written back from a `JSONValue` has `32`, which the
    /// same request shows in the control, and its keys in an order the process's seed gave. The text is
    /// printed, so that two runs, which are two processes, can be held to each other.
    @Test func aReplyComesBackAsTheEngineWroteItWithItsNumbersAndItsKeysInTheEnginesOrder() async throws {
        try await withEngine("text-order") { engine, _ in
            _ = try await engine.requestJSON("run", params: makeCard)
            let text = try await engine.requestJSON("get", params: askPosition)
            var from = text.startIndex
            for key in positionKeys {
                guard let found = text.range(of: "\"\(key)\":", range: from..<text.endIndex) else {
                    Issue.record("the key `\(key)` is not in the engine's order in \(text)")
                    return
                }
                from = found.upperBound
            }
            for written in [#""time":0.0"#, #""value":[32.0,18.0,0.0]"#] {
                #expect(text.contains(written), "the engine wrote \(written): \(text)")
            }
            print("TEXT-DOOR reply: \(text)")
            let value = try await engine.request("get", try JSONValue(readingJSON: askPosition))
            let rewritten = try value.writtenAsJSON()
            print("TEXT-DOOR rewritten: \(rewritten)")
            #expect(try JSONValue(readingJSON: text) == value, "the text and the value say the same")
            #expect(rewritten.contains("[32,18,0]"), "the control: a value written back drops the fraction")
        }
    }

    /// A real job's text is the engine's own too: its path as the engine wrote it, where a value written back
    /// through `JSONEncoder` escapes every slash, which is the control.
    @Test func aJobAnswersTheEnginesTextAsAValueReadsIt() async throws {
        try await withEngine("text-job") { engine, container in
            _ = try await engine.requestJSON("run", params: makeCard)
            let output = container.appending(path: "card.png").path(percentEncoded: false)
            let params = #"{"comp":"Card","output":"\#(output)","format":"png"}"#
            let job = try await engine.jobJSON("export", params: params)
            let text = try await job.resultJSON
            let frame = container.appending(path: "card_00000.png").path(percentEncoded: false)
            #expect(
                text.hasPrefix(#"{"rendered":[{"comp":"Card","output":"\#(frame)","seconds":"#),
                "the job's text is the engine's report, in its order: \(text)")
            #expect(try await job.resultJSON == text, "the text can be awaited again")
            let value = try await job.result
            #expect(try JSONValue(readingJSON: text) == value, "the value reads the same text")
            #expect(try value.writtenAsJSON().contains(#"\/"#), "the control: a value written back escapes the path")
        }
    }

    /// Parameters that are valid JSON but written as no encoder writes them reach the engine unchanged.
    @Test func parametersReachTheEngineAsTheyWereWritten() async throws {
        try await withEngine("text-params") { engine, _ in
            _ = try await engine.requestJSON("run", params: makeCard)
            let spaced = #"{  "comp" : "Card" ,"layer":"S" ,"path" : "transform/position","path":"transform/scale" }"#
            let text = try await engine.requestJSON("get", params: spaced)
            let reading = "the last of a key written twice is the engine's reading: \(text)"
            #expect(text.contains(#""match":"scale""#), Comment(rawValue: reading))
        }
    }

    /// Text that is not JSON is read and refused by the engine, as `InvalidParams` at every door; Kotlin's
    /// door throws the same row, held by `CoreWordsOnDevice`.
    @Test func textThatIsNotJSONIsTheEnginesInvalidParams() async throws {
        try await withEngine("text-refused") { engine, _ in
            await #expect(throws: notJSONRefusal) { try await engine.requestJSON("info", params: "not json") }
            await #expect(throws: notJSONRefusal) { _ = try await engine.frameJSON(params: "not json", format: .png) }
            let job = try await engine.jobJSON("export", params: "not json")
            await #expect(throws: notJSONRefusal) { try await job.resultJSON }
            await #expect(throws: notJSONRefusal) { try await job.result }
            let truncated = CraftError.InvalidParams(detail: "EOF while parsing an object at line 1 column 1")
            await #expect(throws: truncated) { try await engine.requestJSON("info", params: "{") }
            let empty = CraftError.InvalidParams(detail: "EOF while parsing a value at line 1 column 0")
            await #expect(throws: empty) { try await engine.requestJSON("info", params: "") }
        }
    }

    /// The text forms are the doors' own, so each still answers one kind of method alone.
    @Test func eachTextDoorRefusesAMethodAnotherDoorAnswers() async throws {
        try await withEngine("text-doors") { engine, _ in
            let export = await #expect(throws: CraftError.self) { try await engine.requestJSON("export", params: "{}") }
            let info = await #expect(throws: CraftError.self) {
                try await engine.jobJSON("info", params: "{}").resultJSON
            }
            for (refusal, method) in [(export, "export"), (info, "info")] {
                expectUnsupported(refusal, method: method)
            }
        }
    }

    /// A `request`, `job` or `frame` written with no labels and no type around it names the `JSONValue` form,
    /// which is why the text forms have names of their own: under the same base name each would make such a
    /// reference ambiguous. Each text form is named as plainly by its own name.
    @Test func aBareReferenceStillNamesTheJSONValueForm() async throws {
        try await withEngine("text-bare") { engine, _ in
            let request = engine.request
            let job = engine.job
            let frame = engine.frame
            let requestJSON = engine.requestJSON
            let jobJSON = engine.jobJSON
            let frameJSON = engine.frameJSON

            let made: JSONValue = try await request("run", try JSONValue(readingJSON: makeCard))
            #expect(try JSONValue(readingJSON: made.writtenAsJSON()) == made, "the value form answered a value")
            let text: String = try await requestJSON("get", askPosition)
            #expect(text.contains(#""value":[32.0,18.0,0.0]"#), "the text form answered the engine's text: \(text)")

            let askFrame = #"{"comp":"Card"}"#
            let picture: FrameBytes = try await frame(try JSONValue(readingJSON: askFrame), .png)
            let picturedFromText: FrameBytes = try await frameJSON(askFrame, .png)
            #expect(picture.data == picturedFromText.data, "both forms of `frame` asked for the same picture")

            let jobs = [try await job("info", [:]) { _, _, _ in }, try await jobJSON("info", "{}") { _, _, _ in }]
            for started in jobs {
                expectUnsupported(await #expect(throws: CraftError.self) { try await started.result }, method: "info")
            }
        }
    }
}
