// What an engine does when the app leaves the foreground: its GPU, and an export that is running.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import Testing
import UIKit

private let entering = UIApplication.didEnterBackgroundNotification
private let returning = UIApplication.willEnterForegroundNotification

/// An engine whose handle is a fake, listening to the host's notifications under `policy`, and the host.
@MainActor
private func controlled(
    _ policy: BackgroundPolicy = .cpuWhenBackgrounded,
    over fake: FakeEngine = FakeEngine()
) -> (engine: EffectsEngine, host: FakeHost, fake: FakeEngine) {
    let host = FakeHost()
    return (EffectsEngine(engine: fake, background: policy, host: host), host, fake)
}

@Suite @MainActor struct BackgroundPolicyTests {
    @Test func theDefaultGivesTheGpuUpInTheBackground() {
        #expect(BackgroundPolicy.default == .cpuWhenBackgrounded)
        #expect(controlled().engine.background.policy == .cpuWhenBackgrounded)
    }

    @Test func theGpuIsGivenUpOnEnteringTheBackgroundAndAskedForOnReturn() async throws {
        let (engine, host, fake) = controlled()
        host.post(entering)
        #expect(await eventually { await fake.asked.all.map(\.method) == ["suspendGpu"] })
        host.post(returning)
        #expect(await eventually { await fake.asked.all.map(\.method) == ["suspendGpu", "resumeGpu"] })
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil)
    }

    @Test func aGpuKeptThroughTheBackgroundIsNeverGivenUp() async throws {
        let (engine, host, fake) = controlled(.keepGpu)
        host.post(entering)
        host.post(returning)
        await engine.background.settled()
        try await Task.sleep(for: .milliseconds(100))
        #expect(await fake.asked.all.isEmpty)
    }

    @Test func aHandOverTheEngineRefusesIsKeptForTheApp() async throws {
        let fake = FakeEngine()
        fake.gpuRefusal = CraftError.Internal(detail: "the engine is closed")
        let (engine, host, _) = controlled(over: fake)
        host.post(entering)
        #expect(await eventually { engine.background.lastFailure != nil })
        #expect(engine.background.lastFailure as? CraftError == CraftError.Internal(detail: "the engine is closed"))
    }

    @Test func aCpuNoteAfterTheResumeIsNoFailureAndAttachesNothing() async throws {
        let fake = FakeEngine()
        fake.resumeNote = "no adapter"
        let (engine, host, _) = controlled(over: fake)
        host.post(returning)
        #expect(await eventually { await fake.asked.all.map(\.method) == ["resumeGpu"] })
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil)
    }
}

@Suite @MainActor struct BackgroundExportTests {
    /// An engine whose export never answers by itself: the test settles it.
    private func running(_ policy: BackgroundPolicy = .cpuWhenBackgrounded, granting granted: Bool = true) async throws
        -> (
            engine: EffectsEngine, host: FakeHost, job: ExportJob, finish: @Sendable () -> Void
        )
    {
        let fake = FakeEngine()
        let (stream, finish) = AsyncStream<Void>.makeStream()
        fake.answer = { _, _ in
            for await _ in stream { break }
            return "null"
        }
        let (engine, host, _) = controlled(policy, over: fake)
        host.grant(granted)
        let job = try await engine.export(to: URL(filePath: "/tmp/card.mp4"))
        return (engine, host, job, { finish.yield() })
    }

    @Test func anExportRunsUnderABackgroundTaskThatEndsWithIt() async throws {
        let (_, host, job, finish) = try await running()
        #expect(host.begun == [1], "the task began as the export did, in the foreground")
        #expect(host.ended.isEmpty)
        finish()
        _ = try await job.result
        #expect(await eventually { host.ended == [1] })
    }

    @Test func aRenderOfATemplateRunsUnderOneToo() async throws {
        let fake = FakeEngine()
        let (engine, host, _) = controlled(over: fake)
        let job = try await engine.templates.render(
            URL(filePath: "/tmp/card.ectemplate"), to: URL(filePath: "/tmp/card.mp4"))
        _ = try await job.result
        #expect(host.begun == [1])
        #expect(await eventually { host.ended == [1] })
    }

    /// An export handed to the controller again keeps the one task it holds: the system stops an app
    /// whose task is never ended, and a second task begun for the job would leave its first unended.
    @Test func anExportHandedOverAgainHoldsOneTask() async throws {
        let (engine, host, job, finish) = try await running()
        engine.background.protect(job)
        #expect(host.begun == [1])
        finish()
        _ = try await job.result
        #expect(await eventually { host.ended == host.begun })
    }

    /// The controller watches a job's end through its text, which it never reads as a value: an export the
    /// engine refuses, and one whose answer is not JSON, which only a read as a value would refuse, both
    /// end the task all the same.
    @Test func anExportTheEngineRefusesEndsItsTask() async throws {
        let refusal = CraftError.InvalidParams(detail: "the engine's refusal")
        let fake = FakeEngine()
        fake.answer = { _, _ in throw refusal }
        let (engine, host, _) = controlled(over: fake)
        let job = try await engine.export(to: URL(filePath: "/tmp/card.mp4"))
        await #expect(throws: refusal) { try await job.resultJSON }
        #expect(await eventually { host.ended == [1] })
    }

    @Test func anExportWhoseAnswerIsNotJSONEndsItsTask() async throws {
        let fake = FakeEngine()
        fake.answer = { _, _ in "{" }
        let (engine, host, _) = controlled(over: fake)
        let job = try await engine.export(to: URL(filePath: "/tmp/card.mp4"))
        #expect(try await job.resultJSON == "{")
        #expect(await eventually { host.ended == [1] })
    }

    @Test func aGpuKeptThroughTheBackgroundStillRunsItsExportsUnderATask() async throws {
        let (_, host, job, finish) = try await running(.keepGpu)
        #expect(host.begun == [1])
        finish()
        _ = try await job.result
    }

    @Test func theSystemAboutToStopTheAppCancelsTheExport() async throws {
        let (_, host, job, finish) = try await running()
        #expect(!job.isCancelled)
        host.expire(1)
        #expect(job.isCancelled, "the engine is told to stop before the app is suspended")
        #expect(host.ended.isEmpty, "the task ends with the export, not before it has stopped")
        finish()
        _ = try? await job.result
        #expect(await eventually { host.ended == [1] })
    }

    @Test func anExportTheSystemGrantedNoTimeIsCancelledOnEnteringTheBackground() async throws {
        let (_, host, job, finish) = try await running(granting: false)
        #expect(host.begun.isEmpty)
        #expect(!job.isCancelled, "in the foreground nothing is wrong yet")
        host.post(entering)
        #expect(await eventually { job.isCancelled })
        finish()
    }

    @Test func anExportTheSystemGrantsTimeOnEnteringTheBackgroundIsKept() async throws {
        let (_, host, job, finish) = try await running(granting: false)
        host.grant(true)
        host.post(entering)
        #expect(await eventually { host.begun == [1] })
        #expect(!job.isCancelled)
        finish()
        _ = try await job.result
        #expect(await eventually { host.ended == [1] })
    }
}

/// The host of a real app finds the application by Key-Value Coding, which raises an exception for a key
/// that names nothing, and no Swift code can catch one: where there is no application the host grants no task.
@Suite @MainActor struct UIKitHostTests {
    @Test func aKeyTheApplicationClassDoesNotAnswerGrantsNoTask() {
        let host = UIKitHost(applicationKey: "noSuchApplication")
        #expect(host.beginTask(named: "CraftBridge export") {} == nil)
        host.endTask(1)
    }

    /// The key the host reads the application by is the Objective-C name of `UIApplication.shared`, which the
    /// class answers. The package writes it as text, so the compiler checks it here, in a build no app
    /// extension links: a package's test process has no application, so a wrong key would grant no task
    /// there as the right one does, and only this tells them apart.
    @Test func theApplicationIsReadByTheSharedApplicationsOwnName() {
        #expect(UIKitHost.sharedApplicationKey == #keyPath(UIApplication.shared))
        #expect(UIApplication.responds(to: NSSelectorFromString(UIKitHost.sharedApplicationKey)))
        let host = UIKitHost()
        if let task = host.beginTask(named: "CraftBridge export", onExpiry: {}) {
            host.endTask(task)
        }
    }
}

/// The containers of this suite's rigs, removed when the suite ends.
private let engineExportFolders = "engineexport"

/// The two ends the system leaves an export that is running as the app goes to the background, on a real
/// engine: it finishes under its background task, or it is cancelled before the app is suspended.
@Suite(Removing(name: engineExportFolders)) @MainActor struct BackgroundExportOnTheEngineTests {
    /// The export of `rig`'s comp, started through an engine whose host is `host`, once it is rendering.
    private func rendering(_ rig: Rig, under host: FakeHost) async throws -> (engine: EffectsEngine, job: ExportJob) {
        let engine = EffectsEngine(engine: rig.engine, host: host)
        let first = FirstReport()
        let output = rig.output.appending(path: "card.png")
        let job = try await engine.export("Card", to: output, format: .png, progress: first.handler)
        await first.wait(for: job)
        return (engine, job)
    }

    @Test func anExportRunningAsTheAppEntersTheBackgroundFinishesUnderItsTask() async throws {
        let rig = try await Rig(seconds: 2, folders: .isolated(engineExportFolders))
        let host = FakeHost()
        let (engine, job) = try await rendering(rig, under: host)

        host.post(entering)
        #expect(host.begun == [1], "the export holds a task from its start")
        #expect(host.ended.isEmpty)
        let answer = try await job.result
        guard case .object(let fields) = answer, case .array(let rendered) = fields["rendered"] else {
            Issue.record("the export answered \(answer)")
            return
        }
        #expect(!rendered.isEmpty, "it ran to its end")
        #expect(await eventually { host.ended == [1] }, "the task ended with the export")
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil, "the GPU was given up once the export was done")
        try await rig.finish()
    }

    @Test func anExportTheSystemIsAboutToStopIsCancelledAndEndsItsTask() async throws {
        let rig = try await Rig(seconds: 60, folders: .isolated(engineExportFolders))
        let host = FakeHost()
        let (engine, job) = try await rendering(rig, under: host)

        host.post(entering)
        host.expire(1)
        await #expect(throws: CraftError.Cancelled) { try await job.result }
        #expect(await eventually { host.ended == [1] })
        await engine.background.settled()
        try await rig.finish()
    }
}
