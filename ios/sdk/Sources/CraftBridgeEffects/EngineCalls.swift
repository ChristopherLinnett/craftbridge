// The fixed-shape calls of the EffectCraft API. Each one is a thin call into the one method of the
// engine's method table that answers it, made over the UniFFI bindings. A call whose name a generated
// namespace already takes lives as an extension of that namespace instead: see `Templates.swift`.

import CraftBridge
import CraftBridgeFFI
import Foundation
import QuartzCore

extension EffectsEngine {
    /// Starts an EffectCraft engine over the app's `folders`, asking for the GPU as `gpu` names it, and
    /// applying `background` to it for as long as it lives: by default the engine gives its GPU up when
    /// the app enters the background and asks for it again on return (see ``BackgroundController``).
    ///
    /// The start proves it can write where the folders point and loads what they hold — fonts, models,
    /// staged files — so it is awaited rather than run on the caller's thread.
    public static func start(
        folders: Folders,
        gpu: GpuChoice,
        background: BackgroundPolicy = .default
    ) async throws -> EffectsEngine {
        EffectsEngine(engine: try startEffectsEngine(folders: folders, gpu: gpu), background: background)
    }

    /// Runs the command `command` with `params`, as the engine's `exec` answers it, and returns its reply.
    /// The generated namespaces run their commands through this same call; this one is for a command the
    /// namespaces do not cover.
    public func exec(_ command: String, params: JSONValue = [:]) async throws -> JSONValue {
        try await request("exec", ["command": .string(command), "params": params])
    }

    /// Runs the steps in order through the engine's `run`, as `exec` runs one command, and returns the
    /// reply `run` answers the whole run with.
    public func run(_ steps: [(command: String, params: JSONValue)]) async throws -> JSONValue {
        try await request(
            "run",
            ["steps": .array(steps.map { ["command": .string($0.command), "params": $0.params] })]
        )
    }

    /// Lists the properties of the layer `layer` of `comp`, the active composition when left out, as the
    /// engine's `props` answers: `flat` asks for the leaf properties rather than the tree, `time` reads
    /// them at a time other than the current one, and `depth` bounds how deep the tree goes.
    public func props(
        layer: ItemReference,
        comp: ItemReference? = nil,
        flat: Bool = false,
        time: Double? = nil,
        depth: Int? = nil
    ) async throws -> JSONValue {
        try await request(
            "props",
            given([
                "layer": layer.jsonValue,
                "comp": comp?.jsonValue,
                "flat": .bool(flat),
                "time": time?.jsonValue,
                "depth": depth.map { .number(Double($0)) },
            ])
        )
    }

    /// Reads the property `path` of the layer `layer` of `comp`, as the engine's `get` answers: the
    /// property as a whole, whose value sits under `value`.
    public func get(
        path: String,
        layer: ItemReference,
        comp: ItemReference? = nil,
        at time: Double? = nil
    ) async throws -> JSONValue {
        try await request(
            "get",
            given([
                "layer": layer.jsonValue,
                "path": .string(path),
                "comp": comp?.jsonValue,
                "time": time?.jsonValue,
            ])
        )
    }

    /// Sets the property `path` of the layer `layer` of `comp` to `value` or to `expression`, as the
    /// engine's `set` answers: the property as the engine reads it now, its new value under `value`.
    public func set(
        _ path: String,
        to value: JSONValue? = nil,
        expression: String? = nil,
        layer: ItemReference,
        comp: ItemReference? = nil,
        at time: Double? = nil
    ) async throws -> JSONValue {
        try await request(
            "set",
            given([
                "layer": layer.jsonValue,
                "path": .string(path),
                "value": value,
                "comp": comp?.jsonValue,
                "time": time?.jsonValue,
                "expression": expression?.jsonValue,
            ])
        )
    }

    /// Renders one frame as `format` and answers its bytes: `comp` defaults to the active composition,
    /// the frame to the current time, and the size to the composition's own, which `maxSide` or `scale`
    /// then bound. The bytes arrive as UniFFI carries them, laid out as `format` names: PNG, or rows of
    /// straight or premultiplied pixels.
    public func frame(
        comp: ItemReference? = nil,
        at time: Double? = nil,
        frame frameNumber: Int? = nil,
        maxSide: Double? = nil,
        scale: Double? = nil,
        transparent: Bool = false,
        format: PixelFormat = .rgba8Straight
    ) async throws -> FrameBytes {
        try await frame(
            params: given([
                "comp": comp?.jsonValue,
                "time": time?.jsonValue,
                "frame": frameNumber.map { .number(Double($0)) },
                "maxSide": maxSide?.jsonValue,
                "scale": scale?.jsonValue,
                "transparent": .bool(transparent),
            ]),
            format: format
        )
    }

    /// Draws the frame of `comp` — the active composition when left out — at `time` — the current time
    /// when left out — into `layer`, the app's own `CAMetalLayer`, straight from the GPU. The layer is
    /// attached the first time, and again whenever the engine let it go, as after the app returned from the
    /// background; naming the same layer on every call costs no attach.
    ///
    /// Show a transparent frame over the app's own views by making the layer non-opaque. The frame waits
    /// its turn behind a running export, as every call does, so the app shows its last frame meanwhile.
    ///
    /// - Throws: ``CraftError/Gpu(detail:)`` when the engine renders on the CPU, has no surface or lost
    ///   its device or surface (the next call draws again, once a resized layer is configured), and the
    ///   errors a `frame` of the same request throws.
    @MainActor
    public func preview(_ comp: ItemReference? = nil, into layer: CAMetalLayer, at time: Duration? = nil) async throws {
        try await previewTarget.attach(layer)
        try await handle.present(params: previewRequest(comp: comp, at: time).jsonText())
    }

    /// Ends the preview: the engine gives its surface up, and the next ``preview(_:into:at:)`` attaches the
    /// layer afresh. Ending a preview nobody started is not an error.
    ///
    /// - Throws: ``CraftError/Internal(detail:)`` when the engine has been shut down.
    @MainActor
    public func endPreview() async throws {
        try await previewTarget.detach()
    }

    /// The request `preview` makes of the engine's `present`: the fields of a `frame` request, which are
    /// the composition and the time in seconds, each left out when the caller left it out.
    package func previewRequest(comp: ItemReference?, at time: Duration?) -> JSONValue {
        given([
            "comp": comp?.jsonValue,
            "time": time.map { .number(engineSeconds($0)) },
        ])
    }

    /// Starts an export through the engine's `export`, answering the ``ExportJob`` that carries it:
    /// the composition `comp` — the active one when left out — written to `output` as `format`, which is
    /// a word the render queue knows a format by, since the export renders through the queue. Leaving
    /// `format` out takes the word of `output`'s extension. `progress` hears what the export reports;
    /// `extra` carries the request's other fields the way `export` takes them — `start`, `end`, `fps`,
    /// `bitrate`, `hardware`, and the rest — and the engine checks every key. The export runs under the
    /// engine's ``background`` controller, which keeps it out of suspension and cancels it before the
    /// system stops the app.
    public func export(
        _ comp: ItemReference? = nil,
        to output: URL,
        format: RenderQueueCommands.AddFormat? = nil,
        progress: @escaping ProgressHandler = { _, _, _ in },
        extra: [String: JSONValue] = [:]
    ) async throws -> ExportJob {
        try await job(
            "export", exportRequest(comp: comp, to: output, format: format, extra: extra), progress: progress)
    }

    /// Starts `method` of the method table as a job that reports its progress to `progress`, as an export is
    /// started: every call of the table that reports progress is made through here, so each is started alike
    /// and handed to the engine's ``background`` controller, which keeps it out of suspension and cancels it
    /// before the system stops the app. Package-wide so the conformance tests start the corpus's jobs here too.
    package func job(
        _ method: String,
        _ params: JSONValue,
        progress: @escaping ProgressHandler = { _, _, _ in }
    ) async throws -> ExportJob {
        let job = try ExportJob(engine: handle, method: method, params: params, progress: progress)
        await background.protect(job)
        return job
    }

    /// The request `export` takes, and `template.render` takes under its `output`: the composition
    /// `comp` when one is named, the file `output`, the word of `format` when one is given, over the
    /// request's other fields in `extra`, which never replace these.
    func exportRequest(
        comp: ItemReference?,
        to output: URL,
        format: RenderQueueCommands.AddFormat?,
        extra: [String: JSONValue]
    ) -> JSONValue {
        given(
            [
                "comp": comp?.jsonValue,
                "output": .string(output.path(percentEncoded: false)),
                "format": format?.jsonValue,
            ],
            over: extra
        )
    }

    /// Replays the repro `script` through the engine's `replay` and answers the report. The script is
    /// the line-per-step text the desktop's `replay` reads; `keepGoing` runs the whole of it even when
    /// a step fails. A step that fails is no error of the call: the report says so, its `ok` false and
    /// its first failure named, as the desktop's report does.
    public func replay(_ script: String, keepGoing: Bool = false) async throws -> JSONValue {
        try await request("replay", ["script": .string(script), "keepGoing": .bool(keepGoing)])
    }

    /// Times the render of `comp` through the engine's `timings` and answers the figures: `time` picks
    /// the frame, `scale` the size, `runs` how many passes, `play` how many consecutive frames from
    /// `time` on to time as well, without the layer cache and with it, and `gpu` also times the paths
    /// that ask the GPU. The engine takes `runs` and `play` as whole numbers from 1 to 1000.
    public func timings(
        comp: ItemReference? = nil,
        time: Double? = nil,
        scale: Double? = nil,
        runs: Int? = nil,
        play: Int? = nil,
        gpu: Bool = false
    ) async throws -> JSONValue {
        try await request(
            "timings",
            given([
                "comp": comp?.jsonValue,
                "time": time?.jsonValue,
                "scale": scale?.jsonValue,
                "runs": runs.map { .number(Double($0)) },
                "play": play.map { .number(Double($0)) },
                "gpu": .bool(gpu),
            ])
        )
    }

    /// Answers one `line` of the app's MCP transport with the one line the engine replies with, or nil
    /// for a line that needs no reply.
    public func mcp(_ line: String) async throws -> String? {
        try await handle.mcp(line: line)
    }
}

/// `time` in the seconds the engine reads: its nearest whole nanoseconds over a billion, rounded once,
/// which is what Kotlin's `Duration.toDouble(DurationUnit.SECONDS)` makes of the same duration, so a moment
/// reaches the engine as the same seconds from either platform, and as the seconds a `frame` call names it
/// by. Adding the two halves of `components` as doubles rounds twice and misses the nearest double for
/// many durations: `.milliseconds(9)` would be `0.009000000000000001`. A duration past what an `Int64`
/// counts in nanoseconds, some 292 years, is read in whole seconds: no composition runs that long.
private func engineSeconds(_ time: Duration) -> Double {
    let (seconds, attoseconds) = time.components
    // The two halves carry the same sign, so the nearest nanosecond rounds half away from zero.
    let half: Int64 = attoseconds < 0 ? -500_000_000 : 500_000_000
    let (whole, overflowed) = seconds.multipliedReportingOverflow(by: 1_000_000_000)
    let (nanoseconds, carried) = whole.addingReportingOverflow((attoseconds + half) / 1_000_000_000)
    guard !overflowed, !carried else { return Double(seconds) }
    return Double(nanoseconds) / 1e9
}
