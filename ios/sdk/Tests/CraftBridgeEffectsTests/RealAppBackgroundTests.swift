// The app really entering the background and returning, which only the device runner can arrange: a package's
// tests have no UI to press Home with and no XCTest to automate one, so a test that needs the app in the
// background prints the request the runner gave it and waits for the system's own notifications.

import CraftBridge
import CraftBridgeEffects
import Foundation
import Testing
import UIKit

@Suite(.serialized) @MainActor struct RealAppBackgroundTests {
    @Test(.needsTheDeviceRunner)
    func theAppEntersTheBackgroundAndReturnsWhenTheRunnerAsksAndAnEngineRendersOnItsGpuAgainAfterTheTrip()
        async throws
    {
        // The GPU, so the trip hands a real device over: the policy gives it up in the background and takes
        // it back on the return.
        let (engine, container, shutDown) = try await startedEngine(gpu: .preferred)
        defer { try? FileManager.default.removeItem(at: container) }
        let before = try await adapter(of: engine)
        let trip = BackgroundTrip(seconds: 6)
        #expect(UIApplication.shared.applicationState == .active, "the app starts in the foreground")
        guard case .string = before else {
            Issue.record("the engine renders on the CPU before the trip, so no GPU is handed over")
            return
        }

        trip.ask()
        #expect(await trip.left(), "the app entered the background after the request")
        #expect(await trip.cameBack(), "the app came back to the foreground")
        #expect(UIApplication.shared.applicationState == .active, "the app is active again")
        await engine.background.settled()
        #expect(engine.background.lastFailure == nil, "the engine's hand-over across the trip failed")
        #expect(try await adapter(of: engine) == before, "the engine renders on the GPU it had before the trip")
        try await shutDown()
    }
}
