// Gives an engine's GPU up when the app goes to the background, and keeps a running export out of suspension.

import CraftBridgeFFI
import Foundation
import UIKit
import os

/// Applies a ``BackgroundPolicy`` to one engine for the life of the app, and sees to every export that is
/// running when the app leaves the foreground.
///
/// **The GPU.** Under ``BackgroundPolicy/cpuWhenBackgrounded`` the controller calls ``Engine/suspendGpu()``
/// once the app has entered the background and ``Engine/resumeGpu()`` when it returns, and then attaches
/// the layer of a ``PreviewTarget`` again, since the engine's surface went with its device. Under
/// ``BackgroundPolicy/keepGpu`` it does neither.
///
/// **A running export**, whichever policy, whatever folders the engine was started in. An export holds the
/// pending-export journal's file lock, and iOS terminates an app that is suspended while it holds a lock
/// in a container shared with an extension, as an App Group's is; it also holds the GPU device it took at
/// its start until it ends, whatever ``Engine/suspendGpu()`` drops, and no GPU work may be submitted once
/// the app is suspended. The core cannot see suspension, so every export handed to ``protect(_:)`` runs
/// under a background task from the moment it starts to the moment it ends, which keeps the app out of
/// suspension for as long as the system allows. When the system is about to stop the app the export is
/// cancelled, which the engine honours at its next progress report, and the task ends when the export
/// has: an export that has not stopped by the time the system's grace runs out takes the app with it, as
/// any task that outlasts its time does. Nothing here is conditional on an App Group, since the GPU
/// device is the app's own whatever its folders are.
///
/// ``Engine/suspendGpu()`` waits its turn behind a running export, so the GPU is given up once the
/// export has ended, and the export keeps the device it renders on until then.
@MainActor
public final class BackgroundController {
    /// What the controller does with the GPU.
    public let policy: BackgroundPolicy
    /// The failure of the last GPU hand-over the controller made on its own, which no caller can be
    /// told of: the engine was shut down, or the layer could not be attached again. Nil while the last
    /// one went through.
    public private(set) var lastFailure: (any Error)?

    private let engine: Engine
    private let preview: PreviewTarget
    private let host: any BackgroundHost
    private let lifecycle: Lifecycle
    private var running: [ObjectIdentifier: Running] = [:]
    private let transitions = Turns()

    /// A running export, and the system's grant that keeps the app running for it, when there is one.
    private struct Running {
        let job: ExportJob
        var task: Int?
    }

    /// Applies `policy` to `engine` from now on: it listens for the app entering the background and
    /// returning to the foreground. `preview` is the target whose layer is attached again after the GPU
    /// came back.
    package nonisolated init(
        engine: Engine,
        policy: BackgroundPolicy = .default,
        preview: PreviewTarget,
        host: any BackgroundHost = UIKitHost()
    ) {
        self.engine = engine
        self.policy = policy
        self.preview = preview
        self.host = host
        lifecycle = Lifecycle(center: host.notifications)
        lifecycle.listen(for: self)
    }

    /// Keeps the app out of suspension while `job` runs, and cancels it before the system stops the app.
    /// Every export started through `EffectsEngine.export` is handed here. A job the system grants no
    /// time for is cancelled when the app enters the background, since nothing would keep it out of
    /// suspension then. A job handed here again keeps the task it holds.
    public func protect(_ job: ExportJob) {
        let key = ObjectIdentifier(job)
        // A second task for a job already held would replace the first here, which then never ends, and
        // the system stops an app whose task outlives its time.
        guard running[key] == nil else { return }
        running[key] = Running(job: job, task: begin(for: job))
        Task {
            _ = try? await job.result
            finished(key)
        }
    }

    /// Waits until every hand-over the controller started has answered, so a caller or a test reads the
    /// engine's state once the controller is done with it.
    public func settled() async {
        await transitions.settled()
    }

    /// What the app entering the background asks of the controller: first the exports, at once, then the
    /// GPU, in turn after the hand-over before it.
    package func enteredBackground() async {
        holdExports()
        guard policy == .cpuWhenBackgrounded else { return }
        await next { try await $0.engine.suspendGpu() }
    }

    /// What the app returning to the foreground asks of the controller: a device again, and the layer
    /// back on it.
    package func returnedToForeground() async {
        guard policy == .cpuWhenBackgrounded else { return }
        await next {
            // Nil says the engine holds a device; the note says why it renders on the CPU, which is no
            // failure and leaves nothing to attach a layer to.
            if try await $0.engine.resumeGpu() == nil {
                try await $0.preview.reattach()
            }
        }
    }

    private func begin(for job: ExportJob) -> Int? {
        host.beginTask(named: "CraftBridge export") { job.cancel() }
    }

    private func finished(_ key: ObjectIdentifier) {
        if let task = running.removeValue(forKey: key)?.task {
            host.endTask(task)
        }
    }

    /// Asks again for the time of every export the system granted none, and cancels the ones it still
    /// grants none: they would run on, unprotected, into suspension.
    private func holdExports() {
        for (key, export) in running where export.task == nil {
            if let task = begin(for: export.job) {
                running[key]?.task = task
            } else {
                export.job.cancel()
            }
        }
    }

    /// Runs `step` once the hand-overs before it have answered, so a suspend and a resume reach the
    /// engine in the order the app made them, and keeps its failure.
    private func next(_ step: @escaping @MainActor (BackgroundController) async throws -> Void) async {
        await transitions.run { [weak self] in
            guard let self else { return }
            do {
                try await step(self)
                lastFailure = nil
            } catch {
                lastFailure = error
            }
        }
    }
}

/// The app's lifecycle notifications, handed to a controller that is set once it exists, since a
/// notification can arrive before the controller's initializer has returned and the controller is an actor's.
/// The observers are removed when this is let go, so a controller that goes ends its listening with it.
private final class Lifecycle: @unchecked Sendable {
    private struct Target: @unchecked Sendable {
        weak var controller: BackgroundController?
    }

    private let center: NotificationCenter
    private let target = OSAllocatedUnfairLock(initialState: Target())
    private var tokens: [any NSObjectProtocol] = []

    init(center: NotificationCenter) {
        self.center = center
        tokens = [
            observe(UIApplication.didEnterBackgroundNotification) { await $0.enteredBackground() },
            observe(UIApplication.willEnterForegroundNotification) { await $0.returnedToForeground() },
        ]
    }

    deinit {
        for token in tokens {
            center.removeObserver(token)
        }
    }

    /// Hands the notifications to `controller` from now on.
    func listen(for controller: BackgroundController) {
        target.withLock { $0.controller = controller }
    }

    private func observe(
        _ name: Notification.Name,
        _ event: @escaping @MainActor (BackgroundController) async -> Void
    ) -> any NSObjectProtocol {
        center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
            guard let controller = self?.target.withLock({ $0.controller }) else { return }
            Task { @MainActor in await event(controller) }
        }
    }
}
