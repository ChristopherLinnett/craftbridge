// The export, run as CraftBridge's background recipe for iOS says (plan §25).
//
// iOS 26 and later: the export starts from the user's tap inside a `BGContinuedProcessingTask`, to which it
// reports its progress; the system shows that progress and lets the export run on when the app leaves the
// foreground. iOS 17 to 25: the export continues for the short time iOS grants a backgrounded app, which
// `EffectsEngine.background` takes with a background task of its own for every export. Either way, when the
// system stops the app the engine's journal removes the half-written movie at the next start, and `Studio`
// offers to export again.

import BackgroundTasks
import CraftBridge
import CraftBridgeEffects
import Foundation

/// The system's own task for the export, as far as the export needs it: somewhere to report progress to, a
/// way to hear that the system wants the time back, and a way to say the work has ended.
@MainActor
protocol ExportTask {
    /// Tells the system how far the export has come.
    func report(done: UInt64, total: UInt64)
    /// Runs `handler` when the system is about to take the export's time away.
    func onExpiry(_ handler: @escaping @Sendable () -> Void)
    /// Tells the system the work has ended, well or not.
    func end(success: Bool)
}

/// Runs an export under the system's background grace.
@MainActor
enum ExportRecipe {
    #if DEBUG
        /// Whether the last export ran inside the system's continued processing task, which the spine reports.
        private(set) static var lastRanUnderSystemTask = false
    #endif
    private static var running: ExportJob?
    /// Whether the export starting now was asked to stop before its job began: the tap can come while the
    /// export waits for the system's task, or for the engine to take the job.
    private static var stopAsked = false
    /// The system's task for the export that is running, which its progress is reported to.
    private static var systemTask: (any ExportTask)?

    /// Registers the launch handler of the iOS 26 task, which must happen before the app finishes launching.
    static func registerBackgroundTask() {
        if #available(iOS 26.0, *) {
            ContinuedExportTask.register()
        }
    }

    /// Exports `comp` to `movie` as H.264 on the hardware encoder where the phone has one, calling `progress`
    /// with the fraction of the frames made. The export runs inside the system's continued processing task
    /// when the phone has them (iOS 26 and later) and otherwise under the engine's own background task.
    static func run(
        engine: EffectsEngine,
        comp: ItemReference,
        to movie: URL,
        progress: @escaping @MainActor (Double) -> Void
    ) async throws {
        stopAsked = false
        let task = await beginTask(title: "Exporting the card")
        systemTask = task
        #if DEBUG
            lastRanUnderSystemTask = task != nil
        #endif
        defer { systemTask = nil }
        let job: ExportJob
        do {
            if stopAsked { throw CraftError.Cancelled }
            job = try await engine.export(
                comp, to: movie, format: .h264,
                progress: { done, total, _ in
                    Task { @MainActor in
                        systemTask?.report(done: done, total: total)
                        if total > 0 { progress(Double(done) / Double(total)) }
                    }
                },
                extra: ["hardware": true])
        } catch {
            task?.end(success: false)
            throw error
        }
        running = job
        defer { running = nil }
        if stopAsked { job.cancel() }
        task?.onExpiry { job.cancel() }
        do {
            _ = try await job.result
            task?.end(success: true)
        } catch {
            task?.end(success: false)
            throw error
        }
    }

    /// Cancels the export that is running, or the one still starting; the engine stops a running one at its
    /// next progress report.
    static func cancel() {
        stopAsked = true
        running?.cancel()
    }

    /// The system's task for this export, or nil when the phone has none: before iOS 26, or when the system
    /// refuses to start one, which leaves the export to run under the engine's own background task.
    private static func beginTask(title: String) async -> (any ExportTask)? {
        if #available(iOS 26.0, *) {
            return await ContinuedExportTask.begin(title: title)
        }
        return nil
    }
}

/// The iOS 26 continued processing task of one export.
@available(iOS 26.0, *)
@MainActor
final class ContinuedExportTask: ExportTask {
    private static var waiting: CheckedContinuation<ContinuedExportTask?, Never>?
    /// Which wait is the current one, so a wait's timeout never ends the wait of a later export.
    private static var waits = 0

    private let task: BGContinuedProcessingTask

    private init(_ task: BGContinuedProcessingTask) {
        self.task = task
    }

    /// The identifier the app's Info.plist permits: the bundle's own, with the task's name.
    private static var identifier: String {
        (Bundle.main.bundleIdentifier ?? "") + ".export"
    }

    /// Registers the handler the system calls once it has started a task the app submitted.
    static func register() {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { started in
            MainActor.assumeIsolated {
                guard let task = started as? BGContinuedProcessingTask else { return }
                // A task that arrives after its wait gave up has no export to run: ending it at once is what
                // the system asks of a task nobody uses.
                guard let waiting else { return task.setTaskCompleted(success: false) }
                waiting.resume(returning: ContinuedExportTask(task))
                Self.waiting = nil
            }
        }
    }

    /// Asks the system for a task from the user's tap, and waits for the one it starts. The request fails
    /// when the system cannot start the task at once, since a tap that cannot start its export now is better
    /// told so than queued behind other work.
    static func begin(title: String) async -> ContinuedExportTask? {
        let request = BGContinuedProcessingTaskRequest(
            identifier: identifier, title: title, subtitle: "Making your card")
        request.strategy = .fail
        waits += 1
        let wait = waits
        return await withCheckedContinuation { continuation in
            waiting = continuation
            do {
                try BGTaskScheduler.shared.submit(request)
            } catch {
                waiting = nil
                continuation.resume(returning: nil)
                return
            }
            // A task the system never starts must not hold the export back.
            Task {
                try? await Task.sleep(for: .seconds(5))
                guard waits == wait else { return }
                waiting?.resume(returning: nil)
                waiting = nil
            }
        }
    }

    func report(done: UInt64, total: UInt64) {
        task.progress.totalUnitCount = Int64(clamping: total)
        task.progress.completedUnitCount = Int64(clamping: done)
    }

    func onExpiry(_ handler: @escaping @Sendable () -> Void) {
        task.expirationHandler = handler
    }

    func end(success: Bool) {
        task.setTaskCompleted(success: success)
    }
}
