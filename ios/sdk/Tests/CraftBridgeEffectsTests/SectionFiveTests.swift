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

/// The engine's folders for the walk, a container of the run's own, removed when the suite ends; the walk's
/// files sit inside it, where the engine may write. The name is one, so the making and the removing cannot
/// drift apart.
private let walkFoldersName = "sectionfive"
private let walkFolders = Folders.isolated(walkFoldersName)

@Suite(Removing(name: walkFoldersName)) struct SectionFiveTests {
    @Test func theCallsOfSectionFiveRunOnTheSimulator() async throws {
        let engine = try await EffectsEngine.start(folders: walkFolders, gpu: .preferred)

        // One call proves the handle answers: the engine can list its fonts.
        let fonts = try await engine.exec("text.fonts")
        guard case .object(let home) = fonts else {
            Issue.record("the engine answered its fonts with: \(fonts)")
            return
        }
        #expect(home["families"] != nil)

        // The running example in small, as the corpus's card scenario steps it.
        let dir = URL(filePath: walkFolders.container, directoryHint: .isDirectory)
        let sam = dir.appendingPathComponent("sam.png")
        let ann = dir.appendingPathComponent("ann.png")
        let movie = dir.appendingPathComponent("card.mp4")
        let halted = dir.appendingPathComponent("halted.mp4")
        try picture(.green, to: sam)
        try picture(.blue, to: ann)
        let cardURL = try await makeCardTemplate(engine, in: dir, width: 160, height: 90)
        #expect(FileManager.default.fileExists(atPath: cardURL.path), "the template was saved")

        // The §5 calls themselves, and what they left on the card.
        let card = try await engine.templates.open(cardURL)
        try await card.set("Name", "Sam")
        try await card.set("Photo", media: sam)
        #expect(try await controls(of: card, in: engine)["Name"] == "Sam", "the Name control holds Sam")
        let (red, green, _) = try await middle(of: card, in: engine)
        #expect(green > 200 && red < 50, "the card shows Sam's photo, not the template's")

        // The frame the preview would show, at the card's own size: `preview` onto a layer is
        // PreviewTests'.
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

        // Maya can cancel: a cancel the export reads stops it, answers `Cancelled` and leaves no movie. The
        // cancel is asked from the export's first progress report, because that is where an export reads a
        // cancel — the engine reads the token as each report returns — and every export makes its first
        // report before it writes a single file. So the step holds its rule at its own pace, rather than
        // racing a cancel handed in from outside against an export that can render this card quicker than
        // the ask can reach it, and answer `Ok` while the code is right. What says the export stopped where
        // it read the cancel, rather than answering `Cancelled` after finishing all the same, is that no
        // report came after the one that read it: the engine deletes every file a cancelled export listed,
        // however far it got, so the movie being gone says only that nothing was left behind.
        let halting = CancelFromReport(printedAs: "the halted export")
        let stopping = try await engine.export(card.comp, to: halted, format: .h264, progress: halting.handler)
        halting.take(stopping)
        await #expect(throws: CraftError.Cancelled) { try await stopping.result }
        let readBy = try #require(
            halting.lastReport(), "the halted export's cancel reached it at none of its \(halting.count()) reports")
        #expect(
            halting.count() <= readBy,
            "the halted export made \(halting.count() - readBy) reports after the one that read its cancel: the cancel did not stop its work"
        )
        // The cancel was read at the export's first report, which every export makes before it writes a
        // file: read at a later one, it could have missed an export that ends before that report, and the
        // step would be racing again without saying so. The report's place says so on any machine, where
        // the frames it counted cannot: a render's first reports count none either.
        #expect(readBy == 1, "the halted export read its cancel at its report \(readBy), not at its first")
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
