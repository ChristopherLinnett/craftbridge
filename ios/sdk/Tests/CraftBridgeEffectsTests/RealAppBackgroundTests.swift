// The app really entering the background and returning, which only the device runner can arrange: a package's
// tests have no UI to press Home with and no XCTest to automate one, so a test that needs the app in the
// background prints the request the runner gave it and waits for the system's own notifications.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import UIKit

/// The notifications the system posts as the app leaves the foreground and comes back, counted.
@MainActor
private final class Transitions {
    private(set) var entered = 0
    private(set) var returned = 0
    private var observers: [any NSObjectProtocol] = []

    init() {
        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) {
                _ in MainActor.assumeIsolated { self.entered += 1 }
            },
            center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { self.returned += 1 }
            },
        ]
    }

    /// Stops counting.
    func stop() { observers.forEach(NotificationCenter.default.removeObserver) }
}

/// Waits until `condition` holds or `seconds` have passed, which is the time a launch of the runner's second
/// app and the return to the first take.
@MainActor
private func within(seconds: Int, _ condition: () -> Bool) async -> Bool {
    for _ in 0..<(seconds * 20) {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(50))
    }
    return condition()
}

/// What the device runner puts in the app's environment: the line a test prints to ask it to send the app to
/// the background. A run with no runner has none, which leaves the test out.
private let backgroundRequest = ProcessInfo.processInfo.environment["CRAFTBRIDGE_RUNNER_BACKGROUND_REQUEST"]

/// The adapter the engine renders on, as its session records it, or nil when it renders on the CPU.
@MainActor
private func adapter(of engine: EffectsEngine) async throws -> JSONValue? {
    try await engine.request("info", [:]).value(at: "/project/renderer/adapter")
}

@Suite(.serialized) @MainActor struct RealAppBackgroundTests {
    @Test(.enabled(if: backgroundRequest != nil, "only the device runner can send the app to the background"))
    func theAppEntersTheBackgroundAndReturnsWhenTheRunnerAsksAndAnEngineRendersOnItsGpuAgainAfterTheTrip()
        async throws
    {
        let request = try #require(backgroundRequest)
        // The GPU, so the trip hands a real device over: the policy gives it up in the background and takes
        // it back on the return.
        let engine = try await EffectsEngine.start(folders: .isolated(), gpu: .preferred)
        let before = try await adapter(of: engine)
        let seen = Transitions()
        defer { seen.stop() }
        #expect(UIApplication.shared.applicationState == .active, "the app starts in the foreground")
        guard case .string = before else {
            Issue.record("the engine renders on the CPU before the trip, so no GPU is handed over")
            return
        }

        print(request)
        fflush(stdout)
        #expect(await within(seconds: 120) { seen.entered == 1 }, "the app entered the background after the request")
        #expect(await within(seconds: 120) { seen.returned == 1 }, "the app came back to the foreground")
        #expect(UIApplication.shared.applicationState == .active, "the app is active again")
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil, "the engine's hand-over across the trip failed")
        #expect(try await adapter(of: engine) == before, "the engine renders on the GPU it had before the trip")
    }
}
