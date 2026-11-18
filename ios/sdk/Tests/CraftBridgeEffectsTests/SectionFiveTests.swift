// The calls of the plan's §5 run for real, on the simulator: the app starts the engine, opens a
// template, fills its Name and Photo controls, draws a frame, exports the card with its progress, and
// cancels an export. The card is made the way Leo would make one: a composition with a text layer and
// imported footage, its controls added and the whole saved as a `.ectemplate`, here by the calls of the
// corpus's card scenario, at §5's 30 frames a second for 6 seconds but a small size. One engine serves
// the whole walk.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing
import UIKit
import os

/// The value a property answer carries: `get` and `set` answer the property as a whole.
private func value(of reply: JSONValue) -> JSONValue? {
    if case .object(let fields) = reply {
        return fields["value"]
    }
    return nil
}

/// Writes a solid-color picture the engine can import, standing in for the photo from the camera.
private func picture(_ color: UIColor, to url: URL) throws {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8))
    let data = renderer.pngData { _ in
        color.setFill()
        UIRectFill(CGRect(x: 0, y: 0, width: 8, height: 8))
    }
    try data.write(to: url)
}

/// The value of each control of `card`'s instance, by the control's name, as the engine reads it. The
/// wrapper composition holds the instance as its one layer, its first.
private func controls(of card: TemplateInstance, in engine: EffectsEngine) async throws -> [String: JSONValue] {
    let answer = try await engine.essential.instance(layer: "#1", extra: ["comp": card.comp.jsonValue])
    guard case .object(let fields) = answer, case .array(let controls)? = fields["controls"] else {
        Issue.record("the instance answered no controls: \(answer)")
        return [:]
    }
    var values: [String: JSONValue] = [:]
    for case .object(let control) in controls {
        if case .string(let name)? = control["name"] {
            values[name] = control["value"] ?? .null
        }
    }
    return values
}

/// The color at the middle of `card`'s frame, where the Photo layer sits, as red, green and blue.
private func middle(of card: TemplateInstance, in engine: EffectsEngine) async throws -> (UInt8, UInt8, UInt8) {
    let shot = try await engine.frame(comp: card.comp, at: 1.0, format: .rgba8Straight)
    let at = (Int(shot.height) / 2 * Int(shot.width) + Int(shot.width) / 2) * 4
    let pixel = [UInt8](shot.data.dropFirst(at).prefix(3))
    guard pixel.count == 3 else {
        Issue.record("the frame holds no middle pixel: \(shot.width)×\(shot.height), \(shot.data.count) bytes")
        return (0, 0, 0)
    }
    return (pixel[0], pixel[1], pixel[2])
}

@Suite struct SectionFiveTests {
    @Test func theCallsOfSectionFiveRunOnTheSimulator() async throws {
        let engine = try await EffectsEngine.start(folders: .appDefault, gpu: .preferred)

        // One call proves the handle answers: the engine can list its fonts.
        let fonts = try await engine.exec("text.fonts")
        guard case .object(let home) = fonts else {
            Issue.record("the engine answered its fonts with: \(fonts)")
            return
        }
        #expect(home["families"] != nil)

        // The running example in small, as the corpus's card scenario steps it — by name, not by id,
        // so nothing here depends on which numbers the session happens to hand out.
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ios1c-\(UUID().uuidString)", conformingTo: .directory)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let photo = dir.appendingPathComponent("photo.png")
        let sam = dir.appendingPathComponent("sam.png")
        let ann = dir.appendingPathComponent("ann.png")
        let cardURL = dir.appendingPathComponent("card.ectemplate")
        let movie = dir.appendingPathComponent("card.mp4")
        let halted = dir.appendingPathComponent("halted.mp4")
        try picture(.red, to: photo)
        try picture(.green, to: sam)
        try picture(.blue, to: ann)
        let steps: [(command: String, params: JSONValue)] = [
            (
                command: "comp.new",
                params: ["name": "Card", "width": 160, "height": 90, "duration": 6, "frameRate": 30]
            ),
            (command: "layer.newSolid", params: ["comp": "Card", "name": "Bg", "color": "#223344"]),
            (command: "layer.newText", params: ["comp": "Card", "text": "Hello", "name": "Name"]),
            (command: "file.import", params: ["paths": [.string(photo.path(percentEncoded: false))]]),
            (command: "layer.addItem", params: ["comp": "Card", "item": "photo.png"]),
            (
                command: "essential.addProperty",
                params: [
                    "comp": "Card", "layer": "Name", "path": "text/sourceText", "name": "Name",
                ]
            ),
            (command: "essential.addMedia", params: ["layer": "photo.png", "name": "Photo"]),
            (
                command: "essential.exportTemplate",
                params: ["comp": "Card", "path": .string(cardURL.path(percentEncoded: false))]
            ),
        ]
        _ = try await engine.run(steps)
        #expect(FileManager.default.fileExists(atPath: cardURL.path), "the template was saved")

        // The §5 calls themselves, and what they left on the card.
        let card = try await engine.templates.open(cardURL)
        try await card.set("Name", "Sam")
        try await card.set("Photo", media: sam)
        #expect(try await controls(of: card, in: engine)["Name"] == "Sam", "the Name control holds Sam")
        let (red, green, _) = try await middle(of: card, in: engine)
        #expect(green > 200 && red < 50, "the card shows Sam's photo, not the template's")

        // The frame the preview would show: `preview` onto a layer is the next issue's; `frame` is
        // this one's, at the card's own size.
        let shot = try await engine.frame(comp: card.comp, at: 1.0, format: .png)
        #expect(shot.width == 160)
        #expect(shot.height == 90)
        #expect(!shot.data.isEmpty)

        // A second card from the same template is a card of its own: setting the first once the second
        // is open, and so the session's active composition, sets the first alone.
        let other = try await engine.templates.open(cardURL)
        try await card.set("Name", "Ann")
        try await card.set("Photo", media: ann)
        #expect(try await controls(of: card, in: engine)["Name"] == "Ann", "the first card took the new name")
        #expect(try await controls(of: other, in: engine)["Name"] == "Hello", "the second card kept its own")
        let (_, _, blue) = try await middle(of: card, in: engine)
        let (otherRed, _, otherBlue) = try await middle(of: other, in: engine)
        #expect(blue > 200, "the first card shows Ann's photo")
        #expect(otherRed > 200 && otherBlue < 50, "the second card still shows the template's photo")

        // The export, its progress heard as §5's progress bar hears it.
        let reports = OSAllocatedUnfairLock<[(done: UInt64, total: UInt64)]>(initialState: [])
        let job = try await engine.export(
            card.comp,
            to: movie,
            format: .h264,
            progress: { done, total, _ in reports.withLock { $0.append((done, total)) } }
        )
        let answer = try await job.result
        guard case .object(let fields) = answer, case .array(let rendered)? = fields["rendered"] else {
            Issue.record("the export answered no render: \(answer)")
            return
        }
        #expect(rendered.count == 1)
        let size = (try? FileManager.default.attributesOfItem(atPath: movie.path)[.size] as? Int) ?? 0
        #expect(size > 0, "the export wrote the movie")
        let heard = reports.withLock { $0 }
        #expect(!heard.isEmpty, "the export reported its progress")
        // The engine reports at once and then on a beat, so how many reports a short export makes is the
        // machine's to say; the first can come before the render has counted its frames, as 0 of 0, and
        // every report that has counted them counts §5's 180.
        #expect(heard.allSatisfy { $0.done <= $0.total }, "each report is done of total: \(heard)")
        #expect(heard.allSatisfy { $0.total == 0 || $0.total == 180 }, "the export counts §5's 180 frames: \(heard)")

        // Maya can cancel: the export stops, answers `Cancelled` and leaves no half-written movie.
        let stopping = try await engine.export(card.comp, to: halted, format: .h264)
        stopping.cancel()
        await #expect(throws: CraftError.Cancelled) { try await stopping.result }
        #expect(!FileManager.default.fileExists(atPath: halted.path), "the cancelled export left no movie")

        // The property calls answer on a live layer, each the value the last left.
        let named = try await engine.get(path: "text/sourceText", layer: "Name", comp: "Card")
        #expect(value(of: named) == "Hello", "the get answered the property and its value")
        let set = try await engine.set("text/sourceText", to: "Bye", layer: "Name", comp: "Card")
        #expect(value(of: set) == "Bye", "the set answers the property at its new value")
        let again = try await engine.get(path: "text/sourceText", layer: "Name", comp: "Card")
        #expect(value(of: again) == "Bye", "the layer holds what the set left")
        let tree = try await engine.props(layer: "Name", comp: "Card")
        guard case .object = tree else {
            Issue.record("the properties answered no tree: \(tree)")
            return
        }
        let timings = try await engine.timings(comp: "Card", time: 0.5, runs: 1, play: 2)
        guard case .object(let stats) = timings else {
            Issue.record("the timings answered no figures: \(timings)")
            return
        }
        #expect(stats["runs"] == 1, "the timings answer names its passes")
        guard case .object(let frame)? = stats["frame"], case .array(let layers)? = frame["layers"],
            !layers.isEmpty
        else {
            Issue.record("the timings answered no per-layer figures: \(timings)")
            return
        }
        guard case .object(let play)? = stats["play"], case .object(let uncached)? = play["noCache"] else {
            Issue.record("the timings answered no playback figures: \(timings)")
            return
        }
        #expect(uncached["frames"] == 2, "the playback timed the two frames asked for")

        // The last call takes the session away as it proves `replay`: a repro that starts anew, with
        // a check that only passes if the replay ran the command. A failing step is no error of the
        // call but a report that says so, so the report's own verdict is what proves the run.
        let report = try await engine.replay(
            """
            {"start": "new"}
            {"command": "comp.new", "params": {"name": "R", "width": 64, "height": 64}}
            {"expect": {"query": "comp.info", "params": {"comp": 1}, "pointer": "/name", "equals": "R"}}
            """
        )
        guard case .object(let replayed) = report, case .array(let ran)? = replayed["steps"] else {
            Issue.record("the replay answered no report: \(report)")
            return
        }
        #expect(replayed["ok"] == true, "every step of the repro passed: \(report)")
        #expect(ran.count == 3, "the replay ran the start, the command and the check: \(report)")
    }
}
