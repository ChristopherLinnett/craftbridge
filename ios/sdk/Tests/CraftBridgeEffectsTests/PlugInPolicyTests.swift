// T24 on iOS (plan §28, PD13): the case the desktop runner cannot prove. The corpus's two steps are what
// every platform answers alike; where else a module loads from is each platform's own rule, and on iOS a
// plug-in loads only from the plug-in folder the app names inside its bundle. So a module inside the app's
// container but outside that folder is refused, in the engine's words for a capability the build lacks,
// while a module outside every root the folders derive is the confinement's refusal, `Io`, as on the
// desktop and on Android.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeEffectsConformance
import CraftBridgeFFI
import Foundation
import Testing

/// The name these folders are made under and removed under together, so the making and the removing
/// cannot drift apart.
private let appFoldersName = "t24"

/// Where the app's bundle, its container and the folder outside both are made.
private let appFolders = runFolder(appFoldersName)

@Suite(Removing(name: appFoldersName)) struct PlugInPolicyTests {
    /// The sentence the engine refuses a module from outside the bundle's plug-in folder with, after the
    /// path.
    static let outsideTheBundle =
        "WebAssembly effect plug-ins from outside the app bundle's plug-in folder are not available in this build"

    /// The corpus's probe module, written to `path`: a module with `id` in place of the corpus's, of the
    /// same length, since the module's manifest says how long it is, and a process registers one plug-in
    /// per id.
    private func write(id: String, to path: URL) throws {
        let probe = try String(
            contentsOf: Corpus.file.deletingLastPathComponent().appending(path: "fixtures/probe.wat"), encoding: .utf8)
        try FileManager.default.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
        try probe.replacingOccurrences(of: "conformance", with: id).write(to: path, atomically: true, encoding: .utf8)
    }

    @Test("T24 on iOS: a module outside the bundle's plug-in folder is refused, and one outside every root answers Io")
    func aModuleInTheContainerOutsideTheBundlesPlugInFolderIsRefusedAndOneOutsideEveryRootAnswersIo() async throws {
        let root = appFolders
        let (container, bundle) = (root.appending(path: "container"), root.appending(path: "App.app"))
        let bundled = bundle.appending(path: "PlugIns/probe.wasm")
        let inTheContainer = container.appending(path: "loose/probe.wasm")
        let inTheBundle = bundle.appending(path: "Frameworks/probe.wasm")
        let outside = root.appending(path: "elsewhere/probe.wasm")
        try write(id: "bundledprob", to: bundled)
        for refused in [inTheContainer, inTheBundle, outside] { try write(id: "refusedprob", to: refused) }
        let engine = try await EffectsEngine.start(
            folders: engineFolders(container: container, bundle: bundle).allowingPlugins(in: "PlugIns"),
            gpu: .cpuOnly
        )
        func load(_ path: URL) async throws -> JSONValue {
            try await engine.exec("effect.plugins.load", params: ["path": .string(path.path(percentEncoded: false))])
        }

        // Without a refusal on the other three, a loader that refused everything would pass them.
        let loaded = try await load(bundled)
        #expect(loaded.value(at: "/loaded/0/name") == "Gate Probe", "the bundle's plug-in folder loads: \(loaded)")
        for path in [inTheContainer, inTheBundle] {
            let shown = path.path(percentEncoded: false)
            await #expect(throws: CraftError.Unsupported(detail: "\(shown): \(Self.outsideTheBundle)")) {
                try await load(path)
            }
        }
        let shown = outside.path(percentEncoded: false)
        do {
            let answer = try await load(outside)
            Issue.record("a module outside every root must not load: \(answer)")
        } catch CraftError.Io(let detail) {
            #expect(detail.hasPrefix("cannot read `\(shown)`: it is outside the automation roots"), "\(detail)")
        }
    }
}
