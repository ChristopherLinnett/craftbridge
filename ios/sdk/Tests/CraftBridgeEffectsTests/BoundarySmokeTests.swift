// The boundary's whole surface, called once each on the simulator: what this check must call is the
// manifest the `boundary-surface` generator writes from the distribution library's own UniFFI
// metadata, read beside the corpus the conformance suite already reads, so an export the library
// gains joins the list with no edit here — and a check that never calls it fails naming it, before
// the release does. Each answer is held to what the core says it answers, which is what a wrong
// answer fails on.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeEffectsConformance
import CraftBridgeFFI
import Foundation
import Testing

private let smokeFoldersName = "boundarySmoke"

/// One entry of the manifest: what one call of this check must account for.
private struct BoundarySurface: Decodable {
    struct Entry: Decodable {
        let namespace: String
        let kind: String
        let owner: String?
        let name: String

        /// The callable's id: the owner before the name, as the generator's own ids are built.
        var id: String { owner.map { "\($0).\(name)" } ?? name }
    }

    let surface: [Entry]

    /// The manifest's file: beside the corpus, found the same way Corpus finds it — the copy the
    /// device runner puts in the app where there is no checkout, the checkout everywhere else.
    static var file: URL { Corpus.file.deletingLastPathComponent().appending(path: "generated/boundary-surface.json") }
}

/// The reports an export makes to its listener, taken as they come in, on the engine's own thread.
private final class SmokeHearing: ProgressListener, @unchecked Sendable {
    private let lock = NSLock()
    private var heard: [(done: UInt64, total: UInt64, secondsLeft: Double?)] = []

    func progress(done: UInt64, total: UInt64, secondsLeft: Double?) {
        lock.lock()
        heard.append((done, total, secondsLeft))
        lock.unlock()
    }

    var reports: [(done: UInt64, total: UInt64, secondsLeft: Double?)] {
        lock.lock()
        defer { lock.unlock() }
        return heard
    }
}

/// The calls one run of the check has made, by the id the manifest lists each under.
private final class Calls {
    private(set) var ids: Set<String> = []

    /// Makes `call` and records `id` as made, whether the call answers or throws: a call and its record
    /// are one statement, so a call taken out takes its record with it, and the check fails for the
    /// callable that no statement makes any more.
    func made<T>(_ id: String, _ call: () async throws -> T) async rethrows -> T {
        defer { ids.insert(id) }
        return try await call()
    }

    /// Records the listener's method, the one callable the library calls rather than the check: what the
    /// listener heard is its record.
    func heard(_ id: String) {
        ids.insert(id)
    }
}

/// The frames of the comp the smoke engine makes: the export's own, long enough to beat progress,
/// the count the export checks on both sides use.
private let smokeFrames = 300

@Suite(Removing(name: smokeFoldersName), .timeLimit(.minutes(10)))
struct BoundarySmokeTests {
    @Test func smokeChecksCallEveryCallableOfTheBoundaryAsTheLibrarysOwnMetadataListsThem() async throws {
        let manifest = try JSONDecoder().decode(BoundarySurface.self, from: Data(contentsOf: BoundarySurface.file))
        let calls = Calls()
        let folders = Folders.isolated(smokeFoldersName)
        // Handed over before the start, as the start reads it: an empty list says this engine decodes
        // nothing, which is true of it, and making the call is the point.
        await calls.made("set_device_codecs") { setDeviceCodecs(codecs: []) }
        let engine = try await calls.made("start_effects_engine") {
            try startEffectsEngine(folders: folders, gpu: .cpuOnly)
        }
        do {
            try await callTheSurface(engine, folders: folders, calls: calls)
        } catch {
            try? await engine.shutDown()
            throw error
        }

        let ids = Set(manifest.surface.map(\.id))
        let called = calls.ids
        let missed = manifest.surface.map(\.id).filter { !called.contains($0) }
        #expect(
            missed.isEmpty,
            "this check never made these boundary calls: \(missed.sorted()) — add each and hold its answer"
        )
        let stale = called.sorted().filter { !ids.contains($0) }
        #expect(stale.isEmpty, "this check makes calls the library no longer exports: \(stale)")
    }

    /// Every call the smoke makes, each held to the answer the core gives it, recording the id of each.
    private func callTheSurface(_ engine: Engine, folders: Folders, calls: Calls) async throws {
        // The cancel token: its constructor, and both of its methods. A fresh one is not set, and
        // once it is the engine would stop any call carrying it — nothing here is stopped.
        let token = await calls.made("CancelToken.new") { CancelToken() }
        let setAtFirst = await calls.made("CancelToken.is_cancelled") { token.isCancelled() }
        #expect(!setAtFirst, "a fresh cancel token answered that it was set")
        await calls.made("CancelToken.cancel") { token.cancel() }
        #expect(token.isCancelled(), "a cancelled token answered that it was not set")

        // Recovery: a start that found nothing to clean says so with empty lists, which is what the
        // host's own surface test pins for the same clean start. It answers the same once the engine
        // is ended, at the end below.
        let fresh = try await calls.made("Engine.recovery") { try engine.recovery() }
        #expect(
            fresh.removed.isEmpty && fresh.untouched.isEmpty && fresh.refused.isEmpty && fresh.failed.isEmpty
                && fresh.journal.isEmpty,
            "a clean start's recovery was not empty: \(fresh)"
        )
        // The SDK hands an app the same report, straight through from the boundary.
        let sdk = EffectsEngine(engine: engine, host: FakeHost())
        #expect(try sdk.recovery() == fresh, "the SDK's recovery did not answer what the boundary answered")

        // One request opens the comp every later call of this check draws its frame from.
        _ = try await calls.made("Engine.request") {
            try await engine.request(
                method: "run",
                params: """
                    {"steps":[{"command":"comp.new","params":{
                    "width": 640, "height": 360, "duration": \(Double(smokeFrames) / 30.0), "frameRate": 30}}]}
                    """
            )
        }

        // One frame, asked small: what the core says it answers is the frame scaled so its longer
        // side is the side asked for, in exactly the format the call asked for.
        let frame = try await calls.made("Engine.frame") {
            try await engine.frame(params: #"{"time":0,"maxSide":64}"#, format: .rgba8Straight)
        }
        #expect(frame.format == .rgba8Straight, "the frame came back as \(frame.format)")
        #expect(
            frame.width == 64 && frame.height == 36,
            "the comp is 640x360, so its frame at maxSide 64 is 64x36, not \(frame.width)x\(frame.height)"
        )
        #expect(frame.data.count == 64 * 36 * 4, "the frame's bytes are not one RGBA pixel each: \(frame.data.count)")

        // The preview, from an engine holding nothing: the engine's own answer is no surface, and
        // detaching nothing is fine. iOS has no Android window, so every window call answers so.
        let holding = try await calls.made("Engine.has_preview_surface") { try await engine.hasPreviewSurface() }
        #expect(!holding, "a fresh engine answered that it previews")
        try await calls.made("Engine.detach_preview") { try await engine.detachPreview() }
        await #expect(
            throws: CraftError.Gpu(detail: "no surface"),
            "a present with nothing attached was not answered the core's Gpu \"no surface\""
        ) {
            try await calls.made("Engine.present") { try await engine.present(params: "{}") }
        }
        let noWindow = "this platform has no Android window to preview on"
        await #expect(throws: CraftError.Unsupported(detail: noWindow)) {
            try await calls.made("Engine.attach_window") { try await engine.attachWindow(id: 999_999) }
        }
        // The attach that asks to carry the composition's alpha is Android's too, whatever it asks.
        await #expect(throws: CraftError.Unsupported(detail: noWindow)) {
            try await calls.made("Engine.attach_window_with_alpha") {
                try await engine.attachWindowWithAlpha(id: 999_999, alpha: .premultiplied)
            }
        }
        await #expect(throws: CraftError.Unsupported(detail: noWindow)) {
            _ = try await calls.made("window_address") { try windowAddress(id: 999_999) }
        }
        // The Metal attach is this platform's own: a pointer naming no layer is the core's refusal,
        // not a surface — the engine never gets as far as asking the GPU for one.
        await #expect(throws: CraftError.InvalidParams(detail: "that pointer names no layer")) {
            try await calls.made("attach_metal_layer") { try await attachMetalLayer(engine: engine, layer: 0) }
        }

        // The GPU: an engine started for the CPU suspends and resumes, and the resume answers the
        // CPU's own reason, which is what the core says it answers when no device came.
        try await calls.made("Engine.suspend_gpu") { try await engine.suspendGpu() }
        let note = try await calls.made("Engine.resume_gpu") { try await engine.resumeGpu() }
        #expect(
            note == "the app asked for CPU-only rendering",
            "resuming a CPU-only engine answered \(String(describing: note)), not the core's note")

        // The error's words come from the core itself: the cancelled call's whole sentence is the
        // core's, and a message variant answers its own message back as the core writes it.
        let cancelled = await calls.made("craft_error_message") { craftErrorMessage(error: .Cancelled) }
        #expect(cancelled == "cancelled", "the core did not call a cancelled call \"cancelled\"")
        #expect(craftErrorMessage(error: .NotFound(detail: "nothing like it is here")) == "nothing like it is here")

        // One MCP line answers its reply, the engine's tools, each named.
        let reply = try await calls.made("Engine.mcp") {
            try await engine.mcp(line: #"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        }
        let answered = try #require(reply, "the MCP tools/list answered no reply at all")
        if case .array(let tools)? = try JSONValue(jsonText: answered).value(at: "/result/tools") {
            #expect(
                !tools.isEmpty && tools.allSatisfy { $0.value(at: "/name") != nil },
                "the MCP tools/list answered no named tools: \(answered)")
        } else {
            Issue.record("the MCP tools/list answered no list of tools: \(answered)")
        }

        // The export, with its listener and its token. The core reports on its beat, every progress
        // interval, while there is work: a report before the render has counted its frames says none of
        // none, and an export a fast machine ends inside one beat may never say more, so each report counts
        // nothing yet or this comp's frames, and the work is held by what was written and by the render the
        // call answers, which names a sequence by the file of its first frame.
        let output = URL(filePath: folders.cache).appending(path: "swift-boundary-smoke.png")
        let export = """
            {"output":"\(output.path(percentEncoded: false))","format":"png"}
            """
        let hearing = SmokeHearing()
        let exportAnswer = try await calls.made("Engine.request_with_progress") {
            try await engine.requestWithProgress(
                method: "export", params: export, listener: hearing, cancel: CancelToken())
        }
        let first = URL(filePath: folders.cache).appending(path: "swift-boundary-smoke_00000.png")
        let rendered = try JSONValue(jsonText: exportAnswer)
        #expect(
            rendered.value(at: "/rendered/0/output") == .string(first.path(percentEncoded: false))
                && rendered.value(at: "/rendered/1") == nil,
            "the export did not answer the one render it made, of the sequence starting at \(first): \(exportAnswer)"
        )
        if !hearing.reports.isEmpty {
            calls.heard("ProgressListener.progress")
        }
        let reports = hearing.reports
        #expect(!reports.isEmpty, "the export reported no progress at all")
        #expect(
            reports.allSatisfy { $0.total == 0 || $0.total == UInt64(smokeFrames) },
            "a progress beat counted a different comp than the \(smokeFrames) frames exported"
        )
        #expect(reports.allSatisfy { $0.done <= $0.total }, "a progress beat counted done past total")
        let written = try FileManager.default.contentsOfDirectory(atPath: folders.cache).filter {
            $0.hasPrefix("swift-boundary-smoke_") && $0.hasSuffix(".png")
        }
        #expect(written.count == smokeFrames, "the export wrote \(written.count) of \(smokeFrames) frames")
        for name in written {
            try? FileManager.default.removeItem(atPath: "\(folders.cache)/\(name)")
        }

        // Ending is not an error twice, every call after it is refused in the core's words, and recovery
        // is what the start already did: it answers the same once the engine is ended, because the ending
        // changes nothing of it.
        try await calls.made("Engine.shut_down") { try await engine.shutDown() }
        try await engine.shutDown()
        await #expect(
            throws: CraftError.Internal(detail: "engine is closed"),
            "a call after the end was not refused in the core's Internal \"engine is closed\""
        ) {
            try await engine.request(method: "info", params: "{}")
        }
        #expect(
            try engine.recovery() == fresh, "recovery did not answer the same after the end as it answered at the start"
        )
    }
}
