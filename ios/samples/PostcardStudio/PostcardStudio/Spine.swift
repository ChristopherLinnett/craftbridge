// The validation spine of plan §27 as a launch argument of a Debug build, so the app can be driven with no one at
// the screen: `--postcard-spine` makes a photo, names the card for Sam, previews it at 2 s, exports the 6 s movie,
// opens the share sheet on it and closes the sheet again, sharing nothing, and reports each step on the standard
// error, where `devicectl --console` and `simctl launch --console` read it. It is a test hook, so only a Debug
// build holds it and the Release build an app ships has no such argument. The app keeps running afterwards so
// the screen can be looked at.

#if DEBUG
    import AVFoundation
    import CraftBridge
    import Foundation
    import UIKit

    /// The steps of the spine, which are the screen's own calls.
    enum Spine {
        /// How long the share sheet stays open, so the screen can be looked at with it shown.
        private static let sharingShown = Duration.seconds(5)

        /// Says a line of the spine's report on the standard error, which no buffer holds back: a console that
        /// is not a terminal would keep a line on the standard output until the app ended, and the app never ends.
        private static func report(_ line: String) {
            FileHandle.standardError.write(Data("POSTCARD-SPINE \(line)\n".utf8))
        }

        /// Runs the spine on `studio` when the app was launched with `--postcard-spine`.
        @MainActor
        static func runIfAsked(on studio: Studio) async {
            guard ProcessInfo.processInfo.arguments.contains("--postcard-spine") else { return }
            do {
                try await run(on: studio)
            } catch {
                report("failed: \(error.localizedDescription)")
            }
        }

        @MainActor
        private static func run(on studio: Studio) async throws {
            while studio.stage == .starting || studio.previewLayer == nil {
                try await Task.sleep(for: .milliseconds(100))
            }
            guard studio.stage == .ready else { throw SpineError.notReady(String(describing: studio.stage)) }
            studio.name = "Sam"
            studio.time = 2
            await studio.choosePhoto { try picture() }
            await studio.refresh()
            report("previewed at 2 s: \(studio.note ?? "no note")")
            await studio.export()
            guard case .exported(let movie) = studio.stage else {
                throw SpineError.notExported(String(describing: studio.stage))
            }
            let track = try await AVURLAsset(url: movie).loadTracks(withMediaType: .video).first
            let size = try await track?.load(.naturalSize)
            let seconds = try await track?.load(.timeRange).duration.seconds
            let frames = try await track?.load(.nominalFrameRate)
            report("the export ran inside the system's task: \(ExportRecipe.lastRanUnderSystemTask)")
            let (width, height) = (size?.width ?? 0, size?.height ?? 0)
            let bytes = (try? movie.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            report(
                "exported \(movie.lastPathComponent): \(width)x\(height), \(seconds ?? 0) s, \(frames ?? 0) fps, \(bytes) bytes"
            )
            studio.share()
            try await waitForShareSheet(shown: true)
            report("the share sheet offers \(movie.lastPathComponent)")
            try await Task.sleep(for: sharingShown)
            studio.sharing = false
            try await waitForShareSheet(shown: false)
            report("the share sheet closed, nothing shared")
        }

        /// Waits until the system's share sheet is on the screen, or off it, failing after ten seconds.
        @MainActor
        private static func waitForShareSheet(shown: Bool) async throws {
            for _ in 0..<100 {
                if shareSheetOnScreen() == shown { return }
                try await Task.sleep(for: .milliseconds(100))
            }
            throw SpineError.shareSheet(shown: shown)
        }

        /// Whether a share sheet is in a window: presented, or held by a controller that is.
        @MainActor
        private static func shareSheetOnScreen() -> Bool {
            let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            return windows.contains { holdsShareSheet($0.rootViewController) }
        }

        @MainActor
        private static func holdsShareSheet(_ controller: UIViewController?) -> Bool {
            guard let controller else { return false }
            if controller is UIActivityViewController, controller.viewIfLoaded?.window != nil { return true }
            return controller.children.contains { holdsShareSheet($0) }
                || holdsShareSheet(controller.presentedViewController)
        }

        /// A picture to stand for the one a person picks.
        @MainActor
        private static func picture() throws -> Data {
            let image = UIGraphicsImageRenderer(size: CGSize(width: 800, height: 600)).image { context in
                UIColor.systemGreen.setFill()
                context.fill(CGRect(x: 0, y: 0, width: 800, height: 600))
                UIColor.systemYellow.setFill()
                context.cgContext.fillEllipse(in: CGRect(x: 250, y: 150, width: 300, height: 300))
            }
            guard let data = image.pngData() else { throw SpineError.noPicture }
            return data
        }
    }

    /// Why the spine could not go on.
    private enum SpineError: Error, LocalizedError {
        case notReady(String)
        case notExported(String)
        case noPicture
        case shareSheet(shown: Bool)

        var errorDescription: String? {
            switch self {
            case .notReady(let stage): "the studio is not ready: \(stage)"
            case .notExported(let stage): "the card was not exported: \(stage)"
            case .noPicture: "the spine's picture could not be made"
            case .shareSheet(let shown): shown ? "the share sheet never showed" : "the share sheet never closed"
            }
        }
    }
#endif
