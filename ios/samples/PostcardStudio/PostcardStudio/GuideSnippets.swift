// The calls the guides show that Postcard Studio itself never makes: an engine with plug-ins or more fonts, a
// preview ended, commands sent by their ids, the font listing, an engine ended, and the generic door a binding
// to another language calls. Every Swift block of the guides is a run of a file this project compiles
// (`cargo xtask guide-samples`), the app's own calls quoted from the app's own files; these few are compiled
// into the Debug build alone, which the gate builds as it builds every configuration, so the Release build an
// app ships holds none of them.

#if DEBUG
    import CraftBridge
    import CraftBridgeEffects
    import Foundation

    /// The engine with WebAssembly plug-ins switched on, loaded from a folder of the app's bundle.
    func startEngineAllowingPlugins(in folder: String) async throws -> EffectsEngine {
        let folders = Folders.appDefault.allowingPlugins(in: folder)
        return try await EffectsEngine.start(folders: folders, gpu: .preferred)
    }

    /// The engine with one more font folder: what the app bundles beside the phone's own families.
    func startEngineWithFonts(_ fonts: URL) async throws -> EffectsEngine {
        var folders = Folders.appDefault
        folders.fonts = [fonts.path]
        return try await EffectsEngine.start(folders: folders, gpu: .preferred)
    }

    /// Lets the previewed layer go, when the app's view goes with it.
    @MainActor
    func stopPreview(_ engine: EffectsEngine) async throws {
        try await engine.endPreview()
    }

    /// Where EffectCraft's desktop would run a script, an app runs the engine's calls itself: one command
    /// through `exec`, with its parameters as JSON.
    func addTitle(_ engine: EffectsEngine, to comp: ItemReference, title: String) async throws -> JSONValue {
        try await engine.exec("essential.addText", params: ["comp": comp.jsonValue, "text": .string(title)])
    }

    /// Several commands in succession, in order: a step that fails stops the run there, and the steps
    /// before it keep what they did.
    func addTitleAndCaption(
        _ engine: EffectsEngine,
        to comp: ItemReference,
        title: String,
        caption: String
    ) async throws -> JSONValue {
        try await engine.run([
            ("essential.addText", ["comp": comp.jsonValue, "text": .string(title)]),
            ("essential.addText", ["comp": comp.jsonValue, "text": .string(caption)]),
        ])
    }

    /// The faces an app may draw with: the phone's own families and the ones it bundles, and nothing else;
    /// `count` says how many there are.
    func faceCount(_ engine: EffectsEngine) async throws -> Int {
        let listing = try await engine.text.fonts()
        guard case .object(let fields) = listing, case .number(let count)? = fields["count"] else { return 0 }
        return Int(count)
    }

    /// Ends the engine, when the app is done with it.
    func end(_ engine: EffectsEngine) async throws {
        try await engine.shutDown()
    }

    /// One call of a binding to another language, which carries it as a method's name with its parameters
    /// as JSON text: the request goes through the generic door in text, and the reply goes back as the
    /// engine wrote it.
    func forward(_ method: String, _ json: String, to engine: EffectsEngine) async throws -> String {
        try await engine.requestJSON(method, params: json)
    }

    /// The same for a method that reports progress, such as `export`: the door answers the job at once,
    /// and the job's text is the method's reply.
    func forward(
        _ method: String,
        _ json: String,
        to engine: EffectsEngine,
        progress: @escaping ProgressHandler
    ) async throws -> String {
        let job = try await engine.jobJSON(method, params: json, progress: progress)
        return try await job.resultJSON
    }

    /// A frame asked with the `frame` request's own keys, answered as the bytes of a PNG.
    func forwardFrame(_ json: String, to engine: EffectsEngine) async throws -> Data {
        try await engine.frameJSON(params: json, format: .png).data
    }

    /// A frame as rows of pixels, for a binding that draws it itself: the layout and the frame are types the
    /// core names, so a binding writes them in declarations of its own.
    func forwardPixels(_ json: String, to engine: EffectsEngine) async throws -> FrameBytes {
        let layout: PixelFormat = .bgra8Premultiplied
        return try await engine.frameJSON(params: json, format: layout)
    }
#endif
