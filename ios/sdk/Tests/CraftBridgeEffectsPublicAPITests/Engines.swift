// Engines as another package starts them: `EffectsEngine.start` over folders of the test's own, and
// `shutDown()` to end them, which is all a package outside this one has.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeEffectsConformance
import Foundation
import Testing

/// A folder of this run's own in the temporary folder, for one engine's container: not made yet, and the
/// test that asks for it removes it. Its name is none the conformance suite's sweep of that folder takes
/// for a run folder of its own, so a run of that suite beside this one leaves it alone. Its path ends in its
/// name and no slash, since the corpus writes a path after it.
func scratchFolder(_ name: String) -> URL {
    FileManager.default.temporaryDirectory.appending(path: "public-api-\(name)-\(UUID().uuidString)")
}

/// The folders of an engine that reads and writes only inside `container`, laid out as a scenario's are, and
/// the container as its bundle too, since iOS loads a plug-in only from the bundle and a scenario that loads
/// one keeps it in its own folder.
func engineFolders(in container: URL) -> Folders {
    engineFolders(container: container, bundle: container)
}

/// Runs `body` on an engine started on the CPU over a folder of its own, which is removed when it ends. The
/// engine is shut down however `body` ends.
func withEngine<Value: Sendable>(
    _ name: String,
    _ body: (_ engine: EffectsEngine, _ container: URL) async throws -> Value
) async throws -> Value {
    let container = scratchFolder(name)
    defer { try? FileManager.default.removeItem(at: container) }
    let engine = try await EffectsEngine.start(folders: engineFolders(in: container), gpu: .cpuOnly)
    do {
        let value = try await body(engine, container)
        try await engine.shutDown()
        return value
    } catch {
        try? await engine.shutDown()
        throw error
    }
}

/// What every call of an engine that was shut down throws: the core's own words for it, which the Kotlin
/// engine's calls answer too, as `InternalErrorException`.
let closed = CraftError.Internal(detail: "engine is closed")

/// Records an issue unless `refusal` is the `Unsupported` an engine answers a method with at a door that does
/// not answer it, whose words name the method asked.
func expectUnsupported(_ refusal: CraftError?, method: String) {
    guard case .Unsupported(let detail)? = refusal else {
        Issue.record("a door that does not answer `\(method)` ended it in \(String(describing: refusal))")
        return
    }
    #expect(detail.hasPrefix("`\(method)` is not answered by "), "the refusal names its method: \(detail)")
}
