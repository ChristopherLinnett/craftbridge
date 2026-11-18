// What each fixed-shape call asks the engine: the method of the engine's table it sends, the keys it
// puts in the params, and the reply it hands back. Everything here runs against the fake handle.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing

/// The command an `exec` request names, if it names one.
private func command(of params: JSONValue) -> String? {
    if case .object(let fields) = params, case .string(let id)? = fields["command"] {
        return id
    }
    return nil
}

/// The template fake: opening answers the composition 12 and the warnings the test names, its wrapper
/// holds layer 3, an import answers `importReply`, and every other command answers `"done"`.
private func templateEngine(
    recording requests: Requests,
    warnings: String = "[]",
    importReply: String = #"{"items": [66]}"#
) -> EffectsEngine {
    let fake = FakeEngine(recording: requests)
    fake.answer = { method, params in
        if method == "template.open" { return #"{"comp": 12, "warnings": \#(warnings)}"# }
        switch command(of: params) {
        case "comp.info": return #"{"layers": [{"id": 3}]}"#
        case "file.import": return importReply
        default: return "\"done\""
        }
    }
    return EffectsEngine(engine: fake)
}

/// The card the template fake opens.
private func openCard(recording requests: Requests = Requests(), importReply: String = #"{"items": [66]}"#)
    async throws -> TemplateInstance
{
    try await templateEngine(recording: requests, importReply: importReply).templates.open(
        URL(fileURLWithPath: "/t/card.ectemplate")
    )
}

/// One media refusal: the fake answers the open and the wrapper well and the import with
/// `importReply`, and the media set fails with exactly `detail` — the engine's own refusal, in its
/// words or in its silence.
private func mediaRefusal(importReply: String, detail: String) async throws {
    let card = try await openCard(importReply: importReply)
    await #expect(
        throws: CraftError.CommandFailed(command: "file.import", detail: detail, step: nil)
    ) {
        try await card.set("Photo", media: URL(fileURLWithPath: "/t/sam.png"))
    }
}

@Suite struct EngineCallTests {
    @Test func execSendsTheExecMethodWithItsCommand() async throws {
        let requests = Requests()
        let reply = try await engine(recording: requests).exec("layer.newText", params: ["text": "Hi"])
        #expect(reply == "done")
        #expect(await requests.method() == "exec")
        #expect(await requests.params() == ["command": "layer.newText", "params": ["text": "Hi"]])
    }

    @Test func runSendsTheRunMethodWithItsSteps() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).run([
            (command: "comp.new", params: ["name": "A"]),
            (command: "layer.newSolid", params: ["comp": 1]),
        ])
        #expect(await requests.method() == "run")
        let expected: JSONValue = [
            "steps": [
                ["command": "comp.new", "params": ["name": "A"]],
                ["command": "layer.newSolid", "params": ["comp": 1]],
            ]
        ]
        #expect(await requests.params() == expected)
    }

    @Test func propsSendsItsOwnMethodAndOnlyNamedKeys() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).props(layer: 3)
        #expect(await requests.method() == "props")
        #expect(await requests.params() == ["layer": 3, "flat": false])
    }

    @Test func getSendsItsOwnMethodAndThePropertyParams() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).get(path: "text/sourceText", layer: "Name")
        #expect(await requests.method() == "get")
        #expect(await requests.params() == ["layer": "Name", "path": "text/sourceText"])
    }

    @Test func setSendsTheValueOrTheExpression() async throws {
        let requests = Requests()
        let engine = engine(recording: requests)
        _ = try await engine.set("text/sourceText", to: "Sam", layer: "Name")
        _ = try await engine.set("position", expression: "[50, 50]", layer: 2)
        #expect(await requests.method(1) == "set")
        #expect(await requests.params(0) == ["layer": "Name", "path": "text/sourceText", "value": "Sam"])
        #expect(await requests.params(1) == ["layer": 2, "path": "position", "expression": "[50, 50]"])
    }

    @Test func frameSendsTheParamsTextAndTheFormatSeparately() async throws {
        let fake = FakeEngine()
        fake.frameReply = FrameBytes(width: 7, height: 5, format: .png, data: Data([1]))
        let bytes = try await EffectsEngine(engine: fake).frame(
            at: 1.0,
            maxSide: 64,
            transparent: true,
            format: .png
        )
        #expect(bytes == fake.frameReply)
        #expect(await fake.asked.method() == "frame")
        #expect(
            await fake.asked.params() == [
                "params": ["time": 1, "maxSide": 64, "transparent": true], "format": "png",
            ])
    }

    @Test func exportRunsTheExportMethodAndCarriesTheReply() async throws {
        let fake = FakeEngine()
        fake.answer = { _, _ in #"{"rendered": [{"output": "/tmp/out.mp4"}]}"# }
        let job = try await EffectsEngine(engine: fake).export(
            7,
            to: URL(fileURLWithPath: "/tmp/out.mp4"),
            format: .h264,
            extra: ["hardware": true]
        )
        let answer = try await job.result
        #expect(await fake.asked.method() == "export")
        #expect(
            await fake.asked.params() == ["hardware": true, "comp": 7, "output": "/tmp/out.mp4", "format": "h264"]
        )
        #expect(answer == ["rendered": [["output": "/tmp/out.mp4"]]])
    }

    @Test func replaySendsTheScriptAsOneText() async throws {
        let requests = Requests()
        _ = try await engine(recording: requests).replay("{\"start\": \"new\"}", keepGoing: true)
        #expect(await requests.method() == "replay")
        #expect(await requests.params() == ["script": "{\"start\": \"new\"}", "keepGoing": true])
    }

    @Test func timingsSendsItsOwnMethodAndOnlyNamedKeys() async throws {
        let requests = Requests()
        let engine = engine(recording: requests)
        _ = try await engine.timings(runs: 1)
        _ = try await engine.timings(time: 0.5, play: 12, gpu: true)
        #expect(await requests.method() == "timings")
        #expect(await requests.params(0) == ["runs": 1, "gpu": false])
        #expect(await requests.params(1) == ["time": 0.5, "play": 12, "gpu": true])
    }

    @Test func mcpPassesTheLineThroughAndTheReplyBack() async throws {
        let fake = FakeEngine()
        fake.mcpReply = #"{"jsonrpc": "2.0"}"#
        let engine = EffectsEngine(engine: fake)
        let line = #"{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}"#
        #expect(try await engine.mcp(line) == #"{"jsonrpc": "2.0"}"#)
        #expect(await fake.asked.method() == "mcp")
        #expect(await fake.asked.params() == .string(line))
        // A line that needs no reply, such as a notification, answers none.
        fake.mcpReply = nil
        #expect(try await engine.mcp(#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#) == nil)
    }

    @Test func openingATemplateReadsTheWrapperForTheInstance() async throws {
        let requests = Requests()
        let card = try await templateEngine(
            recording: requests,
            warnings: #"["font Avenir is missing"]"#
        ).templates.open(URL(fileURLWithPath: "/t/card.ectemplate"), values: ["Name": "Sam"])
        #expect(await requests.method(0) == "template.open")
        #expect(
            await requests.params(0) == [
                "template": "/t/card.ectemplate",
                "values": ["Name": "Sam"],
            ])
        // The wrapper is read so the instance layer is known for the settings that follow.
        #expect(await requests.params(1) == ["command": "comp.info", "params": ["comp": 12]])
        #expect(card.comp == .number(12))
        #expect(card.warnings == ["font Avenir is missing"])
    }

    @Test func openingLeavesOutNoValuesKeyWhenNoneAreGiven() async throws {
        let requests = Requests()
        _ = try await openCard(recording: requests)
        #expect(await requests.params(0) == ["template": "/t/card.ectemplate"])
    }

    @Test func anOpenAnswerWithoutACompositionIsRefused() async {
        let fake = FakeEngine()
        await #expect(
            throws: CraftError.Internal(detail: "`template.open`: the answer named no composition")
        ) {
            _ = try await EffectsEngine(engine: fake).templates.open(
                URL(fileURLWithPath: "/t/card.ectemplate")
            )
        }
    }

    @Test func anOpenThatLeavesNoInstanceInTheWrapperIsRefused() async {
        let fake = FakeEngine()
        fake.answer = { method, _ in
            method == "template.open" ? #"{"comp": 12}"# : #"{"layers": []}"#
        }
        await #expect(
            throws: CraftError.Internal(
                detail: "`template.open`: the wrapper composition holds no instance"
            )
        ) {
            _ = try await EffectsEngine(engine: fake).templates.open(
                URL(fileURLWithPath: "/t/card.ectemplate")
            )
        }
    }

    /// A setting names the instance's own composition with its layer, so it reaches this card whichever
    /// composition the session has open by then.
    @Test func aPlainSettingNamesTheInstanceItsCompositionAndTheControl() async throws {
        let requests = Requests()
        let card = try await openCard(recording: requests)
        try await card.set("Name", "Sam")
        #expect(
            await requests.params(2) == [
                "command": "essential.set",
                "params": ["comp": 12, "layer": 3, "control": "Name", "value": "Sam"],
            ])
    }

    @Test func aMediaSettingImportsTheFileAndPointsTheControlAtIt() async throws {
        let requests = Requests()
        let card = try await openCard(recording: requests)
        try await card.set("Photo", media: URL(fileURLWithPath: "/t/sam.png"))
        #expect(
            await requests.params(2) == [
                "command": "file.import",
                "params": ["paths": ["/t/sam.png"]],
            ])
        #expect(
            await requests.params(3) == [
                "command": "essential.set",
                "params": ["comp": 12, "layer": 3, "control": "Photo", "item": 66],
            ])
    }

    @Test func aMediaFileTheEngineCannotImportIsRefusedByItsOwnWords() async throws {
        try await mediaRefusal(
            importReply: #"{"errors": ["cannot read the file"]}"#,
            detail: "control `Photo`: cannot import /t/sam.png: \"cannot read the file\""
        )
    }

    @Test func anImportThatAnswersNoFootageIsRefusedByItsOwnSilence() async throws {
        try await mediaRefusal(
            importReply: "{}",
            detail: "control `Photo`: /t/sam.png did not import as footage"
        )
    }

    @Test func renderingSendsTheTemplateMethodWithItsOutputRequest() async throws {
        let fake = FakeEngine()
        let job = try await EffectsEngine(engine: fake).templates.render(
            URL(fileURLWithPath: "/t/card.ectemplate"),
            values: ["Name": "Sam"],
            to: URL(fileURLWithPath: "/tmp/card.mp4"),
            format: .h264
        )
        _ = try await job.result
        #expect(await fake.asked.method() == "template.render")
        let expected: JSONValue = [
            "template": "/t/card.ectemplate",
            "output": ["output": "/tmp/card.mp4", "format": "h264"],
            "values": ["Name": "Sam"],
        ]
        #expect(await fake.asked.params() == expected)
    }

    /// An export's and a render's other fields never replace the file and the format the call types,
    /// as no generated call's `extra` replaces a typed key.
    @Test func anExtraFieldNeverReplacesATypedOne() async throws {
        let fake = FakeEngine()
        let engine = EffectsEngine(engine: fake)
        let stray: [String: JSONValue] = ["output": "/elsewhere.mov", "format": "prores", "fps": 12]
        let card = URL(fileURLWithPath: "/tmp/card.mp4")
        _ = try await engine.export(to: card, format: .h264, extra: stray).result
        _ = try await engine.templates.render(
            URL(fileURLWithPath: "/t/card.ectemplate"), to: card, format: .h264, extra: stray
        ).result
        let request: JSONValue = ["output": "/tmp/card.mp4", "format": "h264", "fps": 12]
        #expect(await fake.asked.params(0) == request)
        #expect(await fake.asked.params(1) == ["template": "/t/card.ectemplate", "output": request])
    }
}
