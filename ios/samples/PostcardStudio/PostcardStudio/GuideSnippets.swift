// The calls the guides show that Postcard Studio itself never makes: an engine with plug-ins or more fonts, a
// preview ended, commands sent by their ids, and the font listing. Every Swift block of the guides is a run of a
// file this project compiles (`cargo xtask guide-samples`), the app's own calls quoted from the app's own files;
// these few are compiled into the Debug build alone, which the gate builds as it builds every configuration, so
// the Release build an app ships holds none of them.

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
#endif
