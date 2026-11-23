// `shutDown()` from outside the package, with the meaning Kotlin's has: every call made after it is refused
// as `Internal`, in the core's words, an export still running is stopped as a cancel stops one, the preview
// goes with the engine, and the folders are free for the next engine once it has answered.

import CraftBridge
import CraftBridgeEffects
import Foundation
import QuartzCore
import Testing

/// A composition to export or to preview, the engine's active one: a reel of `seconds` at 30 frames a second,
/// 1280 by 720, over one solid.
private func makeReel(on engine: EffectsEngine, seconds: Int) async throws {
    let reel: JSONValue = [
        "name": "Reel", "frameRate": 30, "duration": .number(Double(seconds)), "width": 1280, "height": 720,
    ]
    _ = try await engine.exec("comp.new", params: reel)
    _ = try await engine.exec("layer.newSolid", params: ["comp": "Reel", "name": "Ground"])
}

@Suite struct ShutDownTests {
    @Test func everyCallAfterTheEngineIsShutDownIsRefusedAsInternalInTheCoresWords() async throws {
        let container = scratchFolder("after")
        defer { try? FileManager.default.removeItem(at: container) }
        let engine = try await EffectsEngine.start(folders: engineFolders(in: container), gpu: .cpuOnly)
        try await makeReel(on: engine, seconds: 1)
        let recovered = try engine.recovery()

        try await engine.shutDown()

        await #expect(throws: closed) { try await engine.request("info", [:]) }
        await #expect(throws: closed) { _ = try await engine.frame(params: [:], format: .png) }
        await #expect(throws: closed) { try await engine.job("export", [:]).result }
        await #expect(throws: closed) { try await engine.exec("comp.info", params: ["comp": "Reel"]) }
        await #expect(throws: closed) { _ = try await engine.frame(format: .png) }
        await #expect(throws: closed) { try await engine.mcp(#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#) }
        // Ending is no error twice, and what the start recovered is told back as before.
        try await engine.shutDown()
        #expect(try engine.recovery() == recovered)
    }

    @Test func anExportStillRunningIsStoppedAsACancelStopsItAndTheFoldersAreFreeForTheNextEngine() async throws {
        let container = scratchFolder("running")
        defer { try? FileManager.default.removeItem(at: container) }
        let folders = engineFolders(in: container)
        let engine = try await EffectsEngine.start(folders: folders, gpu: .cpuOnly)
        // A minute of frames, so the export has most of them still to draw when the engine ends.
        try await makeReel(on: engine, seconds: 60)
        let recovered = try engine.recovery()
        let frames = container.appending(path: "frames", directoryHint: .isDirectory)

        let (reports, heard) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let job = try await engine.export(
            "Reel", to: frames.appending(path: "reel.png"), format: .png,
            progress: { done, _, _ in
                if done > 0 { heard.yield() }
            })
        // The wait ends at the first report that counts a frame drawn, so the export has written a file by
        // then, or at the export's answer when no such report came, which fails the test instead of leaving
        // it waiting.
        let answered = Task {
            _ = try? await job.result
            heard.finish()
        }
        var drawing = false
        for await _ in reports {
            drawing = true
            break
        }
        #expect(drawing, "the export reported a frame drawn before it ended")
        let written = (try? FileManager.default.contentsOfDirectory(atPath: frames.path(percentEncoded: false))) ?? []
        #expect(!written.isEmpty, "the export had written a frame when the engine was shut down")

        try await engine.shutDown()

        await #expect(throws: CraftError.Cancelled) { try await job.result }
        await answered.value
        let left = (try? FileManager.default.contentsOfDirectory(atPath: frames.path(percentEncoded: false))) ?? []
        #expect(left.isEmpty, "the stopped export left none of the frames it had written: \(left)")

        // The engine has ended by the time `shutDown()` answers: the next one starts over the same folders
        // and finds nothing of the stopped export to clean up.
        let next = try await EffectsEngine.start(folders: folders, gpu: .cpuOnly)
        #expect(try next.recovery() == recovered)
        _ = try await next.request("info", [:])
        try await next.shutDown()
    }

    @Test @MainActor func thePreviewGoesWithTheEngine() async throws {
        let container = scratchFolder("preview")
        defer { try? FileManager.default.removeItem(at: container) }
        let engine = try await EffectsEngine.start(folders: engineFolders(in: container), gpu: .preferred)
        try await makeReel(on: engine, seconds: 1)
        let layer = CAMetalLayer()
        layer.contentsScale = 2
        layer.bounds = CGRect(x: 0, y: 0, width: 32, height: 32)
        try await engine.preview(into: layer)

        try await engine.shutDown()

        await #expect(throws: closed) { try await engine.preview(into: layer) }
        await #expect(throws: closed) { try await engine.endPreview() }
    }
}
