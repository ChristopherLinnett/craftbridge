// What the screen shows and does: the engine, the opened card, its two controls, the preview and the export.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Observation
import QuartzCore

/// The state of Postcard Studio and every step Maya takes (plan §5): start the engine, open the Birthday Card,
/// set its Name and Photo, watch the preview, export and share.
@MainActor
@Observable
final class Studio {
    /// Where the card is, as the screen tells it.
    enum Stage: Equatable {
        /// The engine is starting and the card opening.
        case starting
        /// The engine or the card could not be opened, for the reason shown, so there is nothing to export.
        case unavailable(String)
        /// The card is open and can be changed and exported.
        case ready
        /// The card is being exported, `fraction` of its frames done.
        case exporting(fraction: Double)
        /// The movie is made, at the file, and is offered to share.
        case exported(URL)
        /// A step failed, for the reason shown.
        case failed(String)
    }

    /// The seconds the card lasts.
    static let cardSeconds = 6.0

    /// What the screen shows.
    private(set) var stage = Stage.starting
    /// The name the card greets: the Name control's value.
    var name = "Sam" {
        didSet { if name != oldValue { refreshSoon() } }
    }
    /// The second of the card the preview shows.
    var time = 2.0 {
        didSet { if time != oldValue { refreshSoon() } }
    }
    /// Whether the preview is playing the card through.
    private(set) var playing = false
    /// A note under the preview: the photo's or the preview's own trouble, which stops neither.
    private(set) var note: String?
    /// The layer the engine previews into, once the screen has made its view.
    var previewLayer: CAMetalLayer?
    /// Whether the share sheet is offering the exported movie.
    var sharing = false

    /// Whether the controls can be used: not while the engine starts, and never when it could not.
    var isUsable: Bool {
        switch stage {
        case .starting, .unavailable: false
        default: true
        }
    }

    private var engine: EffectsEngine?
    private var card: TemplateInstance?
    private var photo: URL?
    private var appliedName: String?
    private var appliedPhoto: URL?
    private var refreshing = false
    private var stale = false
    private var player: Task<Void, Never>?

    /// Starts the engine over the app's own folders, asking for the GPU, and opens the Birthday Card the app
    /// bundles. The engine's start also cleans up after an export the system stopped, which is told here.
    func start() async {
        guard engine == nil else { return }
        do {
            let engine = try await EffectsEngine.start(folders: .appDefault, gpu: .preferred)
            guard let template = Bundle.main.url(forResource: "Birthday Card", withExtension: "ectemplate") else {
                throw StudioError.templateMissing
            }
            card = try await engine.templates.open(template)
            self.engine = engine
            let stopped = try engine.recovery().removed
            if !stopped.isEmpty {
                note = "The system stopped an export last time. Export again to finish the card."
            }
            stage = .ready
            refreshSoon()
        } catch {
            stage = .unavailable(describe(error))
        }
    }

    /// Takes the photo picked for the card's Photo control, as the bytes `load` gives: the picker's, which may
    /// fail to arrive (a photo kept only in iCloud, with no network) or arrive in no form the app reads.
    func choosePhoto(_ load: () async throws -> Data?) async {
        do {
            guard let data = try await load() else { throw PostcardPhotoError.unreadable }
            photo = try postcardPhoto(from: data)
            note = nil
            refreshSoon()
        } catch {
            note = describe(error)
        }
    }

    /// Plays the preview through from the second it shows, or stops it.
    func togglePlaying() {
        if playing {
            player?.cancel()
            return
        }
        playing = true
        player = Task {
            let step = 1.0 / 15.0
            while !Task.isCancelled {
                let next = time + step
                time = next > Self.cardSeconds ? 0 : next
                try? await Task.sleep(for: .seconds(step))
            }
            playing = false
        }
    }

    /// Exports the card as an H.264 movie of all its frames, in the background task the recipe shows
    /// (``ExportRecipe``), reporting the progress, and offers the movie to share when it is made.
    func export() async {
        guard let engine, let card else { return }
        player?.cancel()
        stage = .exporting(fraction: 0)
        note = nil
        // The movie is named for the card, never for the Name typed: a Name is free text, whose folder separators
        // would make a path of it and put the movie, under another name, in a folder of the text's making.
        let movie = URL.temporaryDirectory.appending(path: "Birthday Card.mp4")
        do {
            try? FileManager.default.removeItem(at: movie)
            try await ExportRecipe.run(
                engine: engine, comp: card.comp, to: movie,
                progress: { [weak self] fraction in self?.stage = .exporting(fraction: fraction) })
            stage = .exported(movie)
            refreshSoon()
        } catch CraftError.Cancelled {
            stage = .ready
            note = "The export was stopped. Export again to make the card."
            refreshSoon()
        } catch {
            stage = .failed(describe(error))
        }
    }

    /// Cancels the export, whether it is still starting or running, which the engine stops at its next progress
    /// report.
    func cancelExport() {
        ExportRecipe.cancel()
    }

    /// Offers the exported movie in the system's share sheet.
    func share() {
        if case .exported = stage { sharing = true }
    }

    /// Shows the card again once the controls have settled: a change made while a refresh runs is picked up by
    /// the loop's next turn, so typing never queues a refresh for every key.
    private func refreshSoon() {
        stale = true
        guard !refreshing, engine != nil else { return }
        refreshing = true
        Task {
            while stale {
                stale = false
                await refresh()
            }
            refreshing = false
        }
    }

    /// Sets the controls that changed, then draws the preview at the second `time`.
    func refresh() async {
        guard let engine, let card, !isExporting else { return }
        do {
            if appliedName != name {
                try await card.set("Name", .string(name))
                appliedName = name
            }
            if let photo, appliedPhoto != photo {
                try await card.set("Photo", media: photo)
                appliedPhoto = photo
            }
            if let layer = previewLayer {
                try await engine.preview(card.comp, into: layer, at: .seconds(time))
            }
        } catch {
            note = describe(error)
        }
    }

    private var isExporting: Bool {
        if case .exporting = stage { true } else { false }
    }

    /// The words the screen shows for a failure: the SDK's own, which name what went wrong.
    private func describe(_ error: any Error) -> String {
        error.localizedDescription
    }
}

/// A reason Postcard Studio cannot start.
enum StudioError: Error, LocalizedError {
    /// The app was built without its Birthday Card.
    case templateMissing

    var errorDescription: String? {
        "The Birthday Card is not in the app."
    }
}
