// The folders an engine starts in, laid out in an iOS app's own sandbox.

import CraftBridgeFFI
import Foundation

/// The folders an app names when it starts an engine: the one folder the engine writes inside, the
/// app's bundle, which it only reads, and the working folders it keeps inside the first.
public typealias Folders = CraftBridgeFFI.Folders

extension Folders {
    /// The folders of the app's own sandbox, each folder the system gives an app for its kind of file:
    /// settings, models and staged copies in `Library/Application Support/CraftBridge` — staged copies
    /// never under `Caches`, which iOS may empty while the app is not running, since a saved project must
    /// still find the copies it names — what can be thrown away in `Library/Caches/CraftBridge`, the app's
    /// bundle to read, no plug-ins (``allowingPlugins(in:)`` switches them on) and no extra font folder.
    ///
    /// The engine writes only inside the app's sandbox, which holds all of these, so an export may go into
    /// `Documents`, which the Files app shows for an app that shares it, as well as `Library` or `tmp`. The
    /// start proves it can write there through the working folders alone, since on a device the top of the
    /// sandbox takes no new file.
    public static var appDefault: Folders {
        let support = URL.applicationSupportDirectory.appending(path: ownFolder, directoryHint: .isDirectory)
        let working = { (name: String) in path(of: support.appending(path: name, directoryHint: .isDirectory)) }
        return Folders(
            container: path(of: .homeDirectory),
            bundle: path(of: Bundle.main.bundleURL),
            config: working("config"),
            cache: path(of: URL.cachesDirectory.appending(path: ownFolder, directoryHint: .isDirectory)),
            models: working("models"),
            staged: working("staged"),
            plugins: nil,
            fonts: []
        )
    }

    /// These folders with WebAssembly plug-ins switched on, loaded from `folder`, a folder inside the
    /// bundle such as `"PlugIns"`: on iOS the engine loads a plug-in only from a folder the app bundles,
    /// and refuses one anywhere else. Folders with no bundle have nowhere to load one from, so they keep
    /// plug-ins off.
    public func allowingPlugins(in folder: String) -> Folders {
        var folders = self
        folders.plugins = bundle.map { bundle in
            let bundled = URL(filePath: bundle, directoryHint: .isDirectory)
            return path(of: bundled.appending(path: folder, directoryHint: .isDirectory))
        }
        return folders
    }
}

/// The folder of CraftBridge's own in each folder the system gives the app.
private let ownFolder = "CraftBridge"

/// `url`'s path as the engine takes it: the file system's spelling, with no percent-encoding.
private func path(of url: URL) -> String { url.path(percentEncoded: false) }
