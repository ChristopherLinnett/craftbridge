// The baseline of the running example on a phone (plan §9, §29, V5): how long its first preview and its full
// export take through the SDK as an app calls it, the most memory the process held and the threads the export ran
// on, the part of the first preview that is the GPU's start and the path of a preview on a warm engine, and the
// phone's heat, printed on one line that a script reads off by the names `conformance/effects/baseline.toml` gives.
// The test runs only on the acceptance iPhone (`tools/ios-device-test.sh --swift -- --filter RealAppBaselineTests`)
// or where asked for (`CRAFTBRIDGE_BASELINE=1` in the test's own environment, which `xcodebuild test` gives the
// simulator's run as `TEST_RUNNER_CRAFTBRIDGE_BASELINE=1`), alone in an app of its own: beside the gate's other suites
// its times would be a load's, and it would load them.

import CraftBridge
import CraftBridgeEffects
import Foundation
import QuartzCore
import Testing
import UIKit
import os

/// The containers of this suite's engines, removed when the suite ends.
private let baselineFolders = "baseline"

/// The milliseconds a duration of the continuous clock holds.
private func milliseconds(_ duration: Duration) -> Double {
    Double(duration.components.seconds) * 1000 + Double(duration.components.attoseconds) / 1e15
}

/// The card, opened and filled as Maya fills it and drawn at `seconds` into `layer`: the path of a preview.
@MainActor
private func previewedCard(
    on engine: EffectsEngine, template: URL, photo: URL, into layer: CAMetalLayer, at seconds: Double
) async throws -> TemplateInstance {
    let card = try await engine.templates.open(template)
    try await card.set("Name", "Sam")
    try await card.set("Photo", media: photo)
    try await engine.preview(card.comp, into: layer, at: .seconds(seconds))
    return card
}

/// The system's thermal state as its raw value: 0 nominal, 1 fair, 2 serious, 3 critical.
private func thermalState() -> Double { Double(ProcessInfo.processInfo.thermalState.rawValue) }

@Suite(Removing(name: baselineFolders)) @MainActor struct RealAppBaselineTests {
    @Test(.measuresTheBaseline)
    func theRunningExamplesFirstPreviewFullExportAndPeakMemoryAreOnOneLine() async throws {
        let baseline = try Baseline.current()
        let example = baseline.example
        let folders = Folders.isolated(baselineFolders)
        // The app bundles its template: an engine of its own makes it, and is gone before the clock starts.
        let (maker, folder, shutDownMaker) = try await startedEngine(folders: folders)
        let template = try await makeCardTemplate(
            maker, in: folder, width: example.width, height: example.height, seconds: example.seconds,
            frameRate: example.frameRate)
        try await shutDownMaker()
        let sam = folder.appending(path: "sam.png")
        try picture(.green, to: sam)
        // A layer of the card's own size in pixels, as an app's view makes one.
        let layer = CAMetalLayer()
        layer.contentsScale = 3
        layer.bounds = CGRect(x: 0, y: 0, width: example.width / 3, height: example.height / 3)

        let heatBefore = thermalState()
        let clock = ContinuousClock()
        let started = clock.now
        let engine = try await EffectsEngine.start(folders: folders, gpu: .preferred)
        // The first call waits behind the engine thread's own start of the GPU, which ends when it answers.
        _ = try await engine.request("info", [:])
        let gpuStart = milliseconds(started.duration(to: clock.now))
        let card = try await previewedCard(
            on: engine, template: template, photo: sam, into: layer, at: example.previewAt)
        let firstPreview = milliseconds(started.duration(to: clock.now))

        let movie = folder.appending(path: "card.mp4")
        let counted = OSAllocatedUnfairLock<UInt64>(initialState: 0)
        let exporting = clock.now
        let job = try await engine.export(
            card.comp, to: movie, format: .init(rawValue: example.format),
            progress: { _, total, _ in counted.withLock { $0 = total } })
        _ = try await job.result
        let fullExport = milliseconds(exporting.duration(to: clock.now))
        // Read before the checks below load AVFoundation, which no app of the example's does to export, and while
        // the engine that exported still holds its pool.
        let peak = try peakFootprint()
        let threads = try exportThreads()
        let heat = max(heatBefore, thermalState())
        var paths: [Double] = []
        for _ in 0..<example.previewRepeats {
            let began = clock.now
            _ = try await previewedCard(
                on: engine, template: template, photo: sam, into: layer, at: example.previewAt)
            paths.append(milliseconds(began.duration(to: clock.now)))
        }
        try await engine.endPreview()

        #expect(counted.withLock { $0 } == UInt64(example.frames), "the export counts the running example's frames")
        #expect(
            abs(try await seconds(of: movie) - Double(example.seconds)) < 0.1,
            "the movie lasts the running example's \(example.seconds) s")
        let figures: [String: Double] = [
            "first_preview_ms": firstPreview, "gpu_start_ms": gpuStart, "first_path_ms": firstPreview - gpuStart,
            "preview_path_ms": Baseline.median(of: paths),
            "full_export_ms": fullExport, "peak_memory_bytes": peak, "export_threads": threads,
            "thermal_state": heat,
        ]
        let costs = Set(baseline.figures(on: Baseline.platform).filter(\.flagged).map(\.name))
        #expect(
            figures.filter { costs.contains($0.key) }.values.allSatisfy { $0 > 0 }
                && threads > 0 && paths.count == example.previewRepeats,
            "every cost is a measurement: \(figures)")
        let line = try baseline.line(for: figures, repeats: paths)
        print(line)
        fflush(stdout)
        #expect(line.hasPrefix(baseline.line + " first_preview_ms="), "the line starts with the baseline's words")
    }
}
