// What the tests that run in the real app on the acceptance iPhone share: the trip to the background only the
// device runner can arrange, and the pieces a test needs to see what the engine did while it lasted.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing
import UIKit
import os

/// What the device runner puts in the app's environment: the line a test prints to ask it to send the app to
/// the background. A run with no runner has none, which leaves out every test that needs the trip.
let backgroundRequest = ProcessInfo.processInfo.environment["CRAFTBRIDGE_RUNNER_BACKGROUND_REQUEST"]

extension Trait where Self == ConditionTrait {
    /// A test that needs the app really sent to the background and brought back, which a package's tests have
    /// no UI to do: the device runner does it, so the test runs nowhere else.
    static var needsTheDeviceRunner: Self {
        enabled(if: backgroundRequest != nil, "only the device runner can send the app to the background")
    }
}

/// One stay of the app in the background that the device runner arranges when asked, counted by the system's
/// own notifications as the app leaves the foreground and comes back. A trip is complete once the app has
/// entered the background and become active after that, however many of each the system posts: a system
/// alert, for one, takes the app out of the active state and back without a trip, and a count held to exactly
/// one would never hold again.
@MainActor
final class BackgroundTrip {
    private(set) var entered = 0
    private(set) var returned = 0
    /// When the first notice that the app left the foreground came.
    private(set) var enteredAt: Date?
    private let seconds: Int

    /// Counts the notifications from now on; the runner keeps the app away for `seconds` once ``ask()`` is called.
    /// The center keeps the observers for the rest of the run, which count nothing once the trip is let go.
    init(seconds: Int) {
        self.seconds = seconds
        let center = NotificationCenter.default
        center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) {
            [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.entered += 1
                self.enteredAt = self.enteredAt ?? Date()
                self.say("the app entered the background")
            }
        }
        center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) {
            [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                // Becoming active before the app has left is no return from the trip.
                if self.entered > 0 { self.returned += 1 }
                self.say("the app became active")
            }
        }
    }

    /// Says `what` happened, when, and the counts so far, so a trip that does not complete shows why.
    private func say(_ what: String) {
        print("\(what) at \(Date().timeIntervalSinceReferenceDate) (entered \(entered), active \(returned))")
        fflush(stdout)
    }

    /// Asks the runner, with the line it gave and the stay, for the trip to begin.
    func ask() {
        print("\(backgroundRequest ?? "") for \(seconds) seconds")
        fflush(stdout)
    }

    /// Waits until the app has left the foreground, which takes the time the runner needs to launch its second
    /// app. Whether it did.
    func left() async -> Bool {
        await within(seconds: 120) { entered > 0 }
    }

    /// Waits until the app has come back, which takes the stay and the time the runner needs to launch the first
    /// app again. Whether it did.
    func cameBack() async -> Bool {
        await within(seconds: seconds + 120) { returned > 0 }
    }

    /// Whether `ending` came after the app left the foreground. It says the export was still running then
    /// only when the export failed: an answer reaches the main actor some time after the export ends, so the
    /// answer of one that finished just before the app left can come after it. ``framesCarried(by:)`` says
    /// it from the export's own reports.
    func endedAway(_ ending: Ending) -> Bool {
        enteredAt.map { ending.at > $0 } ?? false
    }

    /// How many frames the export `headway` follows still had to make once the app had left the foreground,
    /// as its first report after leaving says: an export that then finishes made them after the app left.
    /// Zero when no report came after the app left, the export having ended first, or when the app never left.
    func framesCarried(by headway: Headway) -> UInt64 {
        guard let enteredAt, let report = headway.first(after: enteredAt) else { return 0 }
        return report.total - min(report.done, report.total)
    }

    /// Waits until the app has left the foreground and come back. Whether it did.
    func completed() async -> Bool {
        let away = await left()
        let back = await cameBack()
        return away && back
    }
}

/// An export's progress reports as they came, so a test can say how far the export had come at a given
/// moment, and a line once a second saying where it is, so a run the system ends shows how far the export had
/// come, and whether it was still moving, when the app was stopped.
final class Headway: Sendable {
    /// One report: when it came, and `done` of `total`.
    struct Report: Sendable {
        let at: Date
        let done: UInt64
        let total: UInt64
    }

    private let kept = OSAllocatedUnfairLock(initialState: (reports: [Report](), said: Date.distantPast))

    /// The handler to give the export.
    var handler: ProgressHandler {
        { [kept] done, total, _ in
            let report = Report(at: Date(), done: done, total: total)
            let due = kept.withLock { kept in
                kept.reports.append(report)
                guard report.at.timeIntervalSince(kept.said) >= 1 else { return false }
                kept.said = report.at
                return true
            }
            guard due else { return }
            print("export progress \(done) of \(total) at \(report.at.timeIntervalSinceReferenceDate)")
            fflush(stdout)
        }
    }

    /// The frames done as the last report at or before `moment` says, or 0 when none came by then.
    func done(at moment: Date) -> UInt64 {
        kept.withLock { $0.reports.last { $0.at <= moment }?.done ?? 0 }
    }

    /// The first report that came after `moment`, or nil when none did.
    func first(after moment: Date) -> Report? {
        kept.withLock { $0.reports.first { $0.at > moment } }
    }
}

/// Waits until `condition` holds or `seconds` have passed.
@MainActor
func within(seconds: Int, _ condition: () -> Bool) async -> Bool {
    for _ in 0..<(seconds * 20) {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(50))
    }
    return condition()
}

/// The real app's host, `UIKitHost`, that notes when each background task begins, is told the system is about
/// to stop the app, and ends, with the time of each.
final class RecordingHost: BackgroundHost, Sendable {
    /// What happened to the tasks, in order.
    struct Event: Sendable {
        let at: Date
        let what: String
    }

    private let real = UIKitHost()
    private let noted = OSAllocatedUnfairLock(initialState: [Event]())

    var notifications: NotificationCenter { real.notifications }

    /// The events so far.
    var events: [Event] { noted.withLock { $0 } }

    /// When the first event named `what` happened, or nil if none did.
    func time(of what: String) -> Date? { events.first { $0.what == what }?.at }

    private func note(_ what: String) {
        let event = Event(at: Date(), what: what)
        noted.withLock { $0.append(event) }
        // Said at once: the system may stop the app before the test reads the list.
        print("background task \(what) at \(event.at.timeIntervalSinceReferenceDate)")
        fflush(stdout)
    }

    @MainActor func beginTask(named name: String, onExpiry: @escaping @Sendable () -> Void) -> Int? {
        let task = real.beginTask(named: name) { [self] in
            note("expired")
            onExpiry()
        }
        note(task == nil ? "refused" : "began")
        return task
    }

    @MainActor func endTask(_ token: Int) {
        note("ended")
        real.endTask(token)
    }
}

/// The adapter the engine renders on, as its session records it, or nil when it renders on the CPU.
@MainActor
func adapter(of engine: EffectsEngine) async throws -> JSONValue? {
    try await engine.request("info", [:]).value(at: "/project/renderer/adapter")
}

/// An engine over `folders`, which no other test shares unless the caller names others, on `gpu`, with the
/// system as `host` has it; the container folder to put its files in, since the engine reads and writes only
/// inside it, which the caller removes; and the call that shuts the engine down.
@MainActor
func startedEngine(
    folders: Folders = .isolated(),
    host: any BackgroundHost = UIKitHost(),
    gpu: GpuChoice = .cpuOnly
) async throws -> (engine: EffectsEngine, container: URL, shutDown: @Sendable () async throws -> Void) {
    let container = URL(filePath: folders.container, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
    let raw = try startEffectsEngine(folders: folders, gpu: gpu)
    return (EffectsEngine(engine: raw, host: host), container, { try await raw.shutDown() })
}

/// The counters `info` reports under `section`, in the order of `names`, each -1 when `info` does not report it.
func infoCounters(of engine: EffectsEngine, in section: String, _ names: [String]) async throws -> [Int] {
    let info = try await engine.request("info", [:])
    return names.map { name in
        guard case .number(let value)? = info.value(at: "/\(section)/\(name)") else { return -1 }
        return Int(value)
    }
}

/// What the hardware encoders and the software encoder behind them have done, as `info.encoding` counts it.
struct Encoding: CustomStringConvertible {
    let hardwarePictures: Int
    let softwarePictures: Int
    let fallbacks: Int
    /// The first frame the software encoder made in the latest switch, or nil before any.
    let lastFallbackFrame: Int?

    init(of engine: EffectsEngine) async throws {
        let counts = try await infoCounters(
            of: engine, in: "encoding", ["hardwarePictures", "softwarePictures", "fallbacks", "lastFallbackFrame"])
        (hardwarePictures, softwarePictures, fallbacks) = (counts[0], counts[1], counts[2])
        lastFallbackFrame = counts[3] < 0 ? nil : counts[3]
    }

    var description: String {
        "\(hardwarePictures) hardware pictures, \(softwarePictures) software pictures, \(fallbacks) fallbacks, "
            + "the latest switch at frame \(lastFallbackFrame.map(String.init) ?? "none")"
    }
}

/// How an export ended, and when: what it answered, or the `CraftError` it threw, which is the only way a job
/// fails.
struct Ending: CustomStringConvertible {
    /// The error the export failed with, or nil when it finished.
    let failure: CraftError?
    /// When the export's answer came.
    let at: Date

    init(_ job: ExportJob) async throws {
        do {
            _ = try await job.result
            failure = nil
        } catch let error as CraftError {
            failure = error
        }
        at = Date()
    }

    var description: String { failure.map { "failed with \($0)" } ?? "finished" }
}
