// Every type the engine's public calls take or answer has a name a package outside this one can write. The
// bindings module that declares them is no product, so the name a caller writes is the core's own: without
// one a caller could pass and read such a value only where the compiler infers its type, and could keep none
// in a declaration of its own, a stored frame, a table of formats, a function that takes a report. This file
// imports the two products' modules alone, so each annotation below compiles only while the core names the
// type.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing

/// The layouts a binding to another language names a frame's bytes by, kept as a table of its own.
private let layouts: [String: PixelFormat] = ["png": .png, "rgba": .rgba8Straight, "bgra": .bgra8Premultiplied]

/// What a binding answers for a frame: its bytes, and its size in whole pixels.
private func reply(of frame: FrameBytes) -> (bytes: Data, width: Int, height: Int) {
    (frame.data, Int(frame.width), Int(frame.height))
}

/// One note of a recovery report as the JSON a binding hands on.
private func json(of note: RecoveryNote) -> JSONValue {
    ["path": .string(note.path), "reason": .string(note.reason)]
}

/// A recovery report as the JSON a binding hands on.
private func json(of report: Recovery) -> JSONValue {
    [
        "removed": .array(report.removed.map(JSONValue.string)),
        "untouched": .array(report.untouched.map(JSONValue.string)),
        "refused": .array(report.refused.map(json(of:))),
        "failed": .array(report.failed.map(json(of:))),
        "journal": .array(report.journal.map(JSONValue.string)),
    ]
}

@Suite struct NamedTypesTests {
    @Test func aCallerKeepsWhatTheEnginesCallsTakeAndAnswerInDeclarationsOfItsOwn() async throws {
        let container = scratchFolder("named")
        defer { try? FileManager.default.removeItem(at: container) }
        let choice: GpuChoice = .cpuOnly
        let engine = try await EffectsEngine.start(folders: engineFolders(in: container), gpu: choice)
        do {
            _ = try await engine.exec("comp.new", params: ["name": "Card", "width": 64, "height": 36])
            let layout = try #require(layouts["bgra"])
            let frame: FrameBytes = try await engine.frame(params: ["comp": "Card"], format: layout)
            let answered = reply(of: frame)
            #expect(frame.format == layout && answered.width == 64 && answered.height == 36)
            #expect(answered.bytes.count == answered.width * answered.height * 4)

            let report: Recovery = try engine.recovery()
            #expect(json(of: report).fields.count == 5, "a start over a new folder still answers a whole report")
        } catch {
            try? await engine.shutDown()
            throw error
        }
        try await engine.shutDown()

        // A frame and a note a caller makes for itself, as a binding's own tests of what it does with one do.
        let made = FrameBytes(width: 1, height: 1, format: .bgra8Premultiplied, data: Data(count: 4))
        #expect(try made.cgImage().width == 1)
        #expect(json(of: RecoveryNote(path: "/a", reason: "a link")) == ["path": "/a", "reason": "a link"])
    }
}
