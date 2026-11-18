// The phone's hardware decoder across a stay of the app in the background: an export that is decoding the
// footage as the app leaves the foreground loses its session, and a session an engine kept from one picture to
// the next fails when the next picture is asked for after the stay. Either way the stream goes on in the
// software decoder, which `info.decoding` counts, and every picture comes out as it did in the foreground, none
// skipped and none twice.

import CraftBridge
import CraftBridgeEffects
import CryptoKit
import Foundation
import Testing
import UIKit

/// The footage the decoder checks decode: a movie a phone's hardware export wrote, played by a composition of
/// the same size and rate, so the engine decodes it picture by picture as an app's playback or export does.
struct Footage {
    let engine: EffectsEngine
    /// The composition that plays the movie.
    let comp = "Play"
    let frames: Int

    /// Writes `seconds` of a turning solid at 960 by 540 and 30 frames a second as H.264 to `movie`, on the
    /// hardware encoder where the phone has one.
    static func write(seconds: Int, to movie: URL, on engine: EffectsEngine) async throws {
        _ = try await engine.run([
            (command: "comp.new", params: size(of: "Clip", seconds: seconds)),
            (command: "layer.newSolid", params: ["comp": "Clip", "name": "Bg", "color": "#c84d21"]),
        ])
        _ = try await engine.set("transform/rotation", expression: "time*90", layer: "Bg", comp: "Clip")
        _ = try await engine.export("Clip", to: movie, format: .h264, extra: ["hardware": true]).result
    }

    /// The parameters of a composition `name` of `seconds` at the footage's size and rate.
    private static func size(of name: String, seconds: Int) -> JSONValue {
        ["name": .string(name), "width": 960, "height": 540, "duration": .number(Double(seconds)), "frameRate": 30]
    }

    /// Imports `movie` into `engine` and plays it in a composition. Each engine decodes the movie with a decoder
    /// session of its own and keeps the pictures it has decoded, so an engine that has played the movie once
    /// decodes nothing when it plays it again.
    init(playing movie: URL, seconds: Int, on engine: EffectsEngine) async throws {
        self.engine = engine
        frames = seconds * 30
        _ = try await engine.run([
            (command: "comp.new", params: Self.size(of: comp, seconds: seconds)),
            (command: "file.import", params: ["paths": [.string(movie.path(percentEncoded: false))]]),
            (command: "layer.addItem", params: ["comp": .string(comp), "item": "footage.mp4"]),
        ])
    }

    /// A fingerprint of each picture of `range`, rendered one after another from the composition.
    func pictures(_ range: Range<Int>) async throws -> [String] {
        var seen: [String] = []
        for index in range {
            seen.append(fingerprint(of: try await engine.frame(comp: .name(comp), frame: index).data))
        }
        return seen
    }

    /// Exports the composition as a sequence of PNGs into `folder`, the whole of it, its progress told to
    /// `progress`.
    func export(into folder: URL, progress: @escaping ProgressHandler = { _, _, _ in }) async throws -> ExportJob {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return try await engine.export(
            .name(comp), to: folder.appending(path: "picture.png"), format: .png, progress: progress)
    }
}

/// SHA-256 of `data`, in hexadecimal.
func fingerprint(of data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

/// A fingerprint of each file of `folder`, by name.
func fingerprints(ofFilesIn folder: URL) throws -> [String: String] {
    var seen: [String: String] = [:]
    for name in try FileManager.default.contentsOfDirectory(atPath: folder.path(percentEncoded: false)) {
        seen[name] = fingerprint(of: try Data(contentsOf: folder.appending(path: name)))
    }
    return seen
}

/// What the hardware decoders have done, as `info.decoding` counts it.
struct Decoding: CustomStringConvertible {
    let sessions: Int
    let pictures: Int
    let declined: Int
    let fallbacks: Int

    init(of engine: EffectsEngine) async throws {
        let info = try await engine.request("info", [:])
        func count(_ name: String) -> Int {
            guard case .number(let value)? = info.value(at: "/decoding/\(name)") else { return -1 }
            return Int(value)
        }
        (sessions, pictures, declined, fallbacks) = (
            count("sessions"), count("pictures"), count("declined"), count("fallbacks")
        )
    }

    var description: String {
        "\(sessions) sessions, \(pictures) pictures, \(declined) declined, \(fallbacks) fallbacks"
    }
}

/// What a foreground engine renders of footage, kept as the reference, and a second engine over the same folders
/// with the footage to play, whose decoder session is its own.
private struct Playback {
    let player: EffectsEngine
    let footage: Footage
    let container: URL
    /// The files of the foreground's export of the footage, by name.
    let exported: [String: String]
    /// The foreground's render of each picture, in order.
    let pictures: [String]
    let shutDown: @Sendable () async throws -> Void

    /// Writes `seconds` of footage, exports it in the foreground as the reference, and starts the second engine.
    /// The foreground's own decode is on the hardware decoder and never falls back, so a fallback a test counts
    /// across a stay in the background is the stay's.
    @MainActor
    init(seconds: Int) async throws {
        let folders = Folders.isolated()
        let writer = try await startedEngine(folders: folders)
        container = writer.container
        let movie = container.appending(path: "footage.mp4")
        try await Footage.write(seconds: seconds, to: movie, on: writer.engine)
        let before = try await Decoding(of: writer.engine)
        let foreground = try await Footage(playing: movie, seconds: seconds, on: writer.engine)
        let kept = container.appending(path: "foreground")
        _ = try await foreground.export(into: kept).result
        exported = try fingerprints(ofFilesIn: kept)
        #expect(exported.count == foreground.frames, "the foreground export wrote every picture")
        pictures = try await foreground.pictures(0..<foreground.frames)
        let after = try await Decoding(of: writer.engine)
        print("the footage in the foreground: \(before) before, \(after) after")
        #expect(after.pictures > before.pictures, "the hardware decoder decoded the foreground's pictures")
        #expect(after.fallbacks == before.fallbacks, "the foreground's decode never fell back to software")
        try await writer.shutDown()

        let started = try await startedEngine(folders: folders)
        player = started.engine
        shutDown = started.shutDown
        footage = try await Footage(playing: movie, seconds: seconds, on: player)
        let info = try await player.request("info", [:])
        #expect(
            info.value(at: "/hardwareDecoding") == true, "the phone decodes in hardware, or there is no session to lose"
        )
    }
}

@Suite(.serialized) @MainActor struct RealAppDecoderTests {
    @Test(.needsTheDeviceRunner)
    func anExportDecodingInHardwareAcrossAStayInTheBackgroundLosesItsSessionAndEveryPictureStillComesOutOnce()
        async throws
    {
        let playback = try await Playback(seconds: 20)
        defer { try? FileManager.default.removeItem(at: playback.container) }
        let before = try await Decoding(of: playback.player)
        let trip = BackgroundTrip(seconds: 10)
        let away = playback.container.appending(path: "background")
        let headway = Headway()
        let job = try await playback.footage.export(into: away, progress: headway.handler)
        trip.ask()
        let ending = try await Ending(job)
        #expect(await trip.completed(), "the app left the foreground and came back")
        let after = try await Decoding(of: playback.player)
        let carried = trip.framesCarried(by: headway)
        print(
            "the export across the trip: \(ending), \(carried) frames still to make when the app left, "
                + "\(before) before, \(after) after")

        #expect(carried > 0, "the export was still rendering once the app had left the foreground")
        #expect(ending.failure == nil, "the export finished across the trip")
        #expect(
            try fingerprints(ofFilesIn: away) == playback.exported,
            "every picture is the foreground's, none skipped and none twice")
        #expect(after.pictures > before.pictures, "the hardware decoder was the one decoding")
        #expect(
            after.fallbacks > before.fallbacks,
            "the session was lost in the background and the stream went on in software")
        try await playback.shutDown()
    }

    @Test(.needsTheDeviceRunner)
    func aSessionTheEngineKeptAcrossAStayInTheBackgroundFailsAndEveryPictureStillComesOutOnce() async throws {
        let playback = try await Playback(seconds: 4)
        defer { try? FileManager.default.removeItem(at: playback.container) }
        let footage = playback.footage
        let middle = footage.frames / 2

        let before = try await Decoding(of: playback.player)
        let trip = BackgroundTrip(seconds: 8)
        var seen = try await footage.pictures(0..<middle)
        trip.ask()
        #expect(await trip.completed(), "the app left the foreground and came back")
        seen += try await footage.pictures(middle..<footage.frames)
        let after = try await Decoding(of: playback.player)
        print("the pictures across the trip: \(before) before, \(after) after")

        #expect(seen == playback.pictures, "every picture is the foreground's, none skipped and none twice")
        #expect(after.pictures > before.pictures, "the hardware decoder was the one decoding")
        #expect(
            after.fallbacks > before.fallbacks, "the session the engine kept failed and the stream went on in software")
        try await playback.shutDown()
    }
}
