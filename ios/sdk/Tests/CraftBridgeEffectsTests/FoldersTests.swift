import CraftBridge
import CraftBridgeFFI
import Foundation
import Testing

/// The suite that starts engines in the app's own folders: `.appDefault` is what these tests are about, so
/// these are the engines that may not move to isolated folders. They run one at a time here, and the suite
/// holds those folders against every other run of the suite on the simulator while they do.
@Suite(.serialized, AppsFoldersLock()) struct FoldersTests {
    /// The suite's tests run inside its hold on the app's own folders, which another run's waits for: a
    /// second open of the folder finds it held.
    @Test func theSuiteHoldsTheAppsFoldersWhileItsTestsRun() throws {
        let other = try openedFolder(AppsFoldersLock.folder)
        defer { release(other) }
        #expect(try !lockedAtOnce(other, AppsFoldersLock.folder), "another run could start in the app's folders now")
    }

    /// Every folder is the sandbox's own folder for its kind of file, and the folder the engine writes
    /// inside is the sandbox itself, which holds every folder it writes and every folder an app writes in.
    @Test func theDefaultFoldersAreTheSandboxsFolderForEachKindOfFile() {
        let folders = Folders.appDefault
        let container = URL(filePath: NSHomeDirectory(), directoryHint: .isDirectory).path(percentEncoded: false)
        #expect(folders.container == container)
        #expect(folders.config == container + "Library/Application Support/CraftBridge/config/")
        #expect(folders.models == container + "Library/Application Support/CraftBridge/models/")
        #expect(folders.staged == container + "Library/Application Support/CraftBridge/staged/")
        #expect(folders.cache == container + "Library/Caches/CraftBridge/")
        #expect(folders.bundle == Bundle.main.bundleURL.path(percentEncoded: false))
        #expect(folders.plugins == nil)
        #expect(folders.fonts.isEmpty)
    }

    @Test func pluginsLoadOnlyFromAFolderInsideTheBundle() throws {
        let folders = Folders.appDefault
        let bundle = try #require(folders.bundle)
        var allowing = folders.allowingPlugins(in: "PlugIns")
        #expect(allowing.plugins == bundle + "PlugIns/")
        allowing.plugins = nil
        #expect(allowing == folders, "switching plug-ins on changes nothing else")
        var unbundled = folders
        unbundled.bundle = nil
        #expect(
            unbundled.allowingPlugins(in: "PlugIns").plugins == nil, "with no bundle there is nowhere to load one from")
    }

    /// The start prepares the folders as the core does for every engine: it makes the working folders, and
    /// proves it can write in each with a probe file, which it removes.
    @Test func theDefaultFoldersPassTheStartsWriteProbe() async throws {
        let folders = Folders.appDefault
        let engine = try startEffectsEngine(folders: folders, gpu: .cpuOnly)
        try await engine.shutDown()
        for folder in [folders.container, folders.config, folders.cache, folders.models, folders.staged] {
            let kind = try URL(filePath: folder).resourceValues(forKeys: [.isDirectoryKey])
            #expect(kind.isDirectory == true, "the start made no folder at \(folder)")
            let names = try FileManager.default.contentsOfDirectory(atPath: folder)
            #expect(!names.contains { $0.hasPrefix(".craftbridge-write-probe-") }, "a probe was left in \(folder)")
        }
    }

    /// The core refuses a working folder outside the folder the engine writes inside, so that folder must
    /// hold the cache as well as the folders in Application Support.
    @Test func aContainerThatLeavesTheCacheOutsideIsRefused() throws {
        var folders = Folders.appDefault
        folders.container = URL.applicationSupportDirectory.path(percentEncoded: false)
        let refusal = try #require(throws: CraftError.self) { try startEffectsEngine(folders: folders, gpu: .cpuOnly) }
        guard case .Io(let detail) = refusal else {
            Issue.record("the start answered \(refusal), not Io")
            return
        }
        let outside = "the cache folder \(folders.cache) is outside the container \(folders.container)"
        #expect(detail == outside + ": the engine writes only inside its container")
    }

    /// The engine writes anywhere in the app's sandbox, so an app exports into `Documents`, which the Files
    /// app shows for an app that shares it, as well as into its `Library`. The engine starts in
    /// `.appDefault` because an export reaching the app's documents is exactly what its write root being
    /// the sandbox is on test for.
    @Test func anExportGoesIntoTheAppsDocuments() async throws {
        let rig = try await Rig(seconds: 1, in: .documentsDirectory, folders: .appDefault)
        let job = try ExportJob(engine: rig.engine, method: "export", params: rig.export)
        await #expect(throws: Never.self) { try await job.result }
        let written = try FileManager.default.contentsOfDirectory(atPath: rig.output.path(percentEncoded: false))
        #expect(written.contains { $0.hasSuffix(".png") }, "the export wrote \(written)")
        try await rig.finish()
    }
}
