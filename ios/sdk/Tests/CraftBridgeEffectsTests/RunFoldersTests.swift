// The run folders and the hold that keep two runs of the suite on one simulator apart, and a run from
// leaving anything behind on it.

import Darwin
import Foundation
import Testing
import os

/// This suite's own run folders.
private let probeFolders = "runfolders"

/// How long a run waits for its sweep to make its folder once the home is free: there only so that a
/// sweep that never ends fails the test, naming what it waited for, rather than holding the whole run
/// until the hosted job's own limit, hours later, with no word of why. Ten minutes is twenty times the
/// 30 s a slow hosted runner once overran, and far more again than the seconds a whole run of the suite
/// waits on a developer's Mac, so no load fails it: it bounds a hang, never the sweep's speed.
private let sweepEndsWithin: TimeInterval = 600

/// Whether `folder` is there.
private func exists(_ folder: URL) -> Bool {
    FileManager.default.fileExists(atPath: folder.path(percentEncoded: false))
}

/// A folder of the home under a run folder's name, made by nothing that holds it.
private func unheld(_ kind: String) throws -> URL {
    let folder = newRunFolder(kind)
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    return folder
}

/// What a test throws to leave a hold early.
private struct Thrown: Error {}

@Suite(Removing(name: probeFolders)) struct RunFoldersTests {
    @Test func aRemovingTakesEveryFolderMadeUnderItsNameAndNoOther() async throws {
        let name = "removing-\(UUID().uuidString)"
        let taken = [runFolder(name), runFolder(name)]
        let kept = runFolder(probeFolders)
        #expect(taken.allSatisfy(exists), "a run folder is there once it is made")
        try await Removing(name: name).provideScope(for: try #require(Test.current), testCase: nil) {}
        #expect(!taken.contains(where: exists), "the suite's folders are gone when it ends")
        #expect(exists(kept), "another suite's folder stays until that suite ends")
    }

    @Test func theSweepRemovesAFolderNoRunHoldsAndKeepsOneAnotherRunHolds() throws {
        let left = try unheld("left")
        // Another run's folder, made and held as a run makes and holds its own, which this run's sweep meets.
        let live = newRunFolder("live")
        let other = try holdingTheHome { try madeAndHeld(live) }
        defer {
            release(other)
            try? FileManager.default.removeItem(at: live)
        }
        _ = runFolder(probeFolders)
        #expect(!exists(left), "a folder no run holds, as a killed run leaves one, is swept")
        #expect(exists(live), "a folder another run holds is kept")
    }

    /// Another run makes its folder and then holds it, both while it holds the home, so this run's sweep
    /// waits for the home rather than meet that folder made and not yet held. The making runs on a queue of
    /// its own, since it blocks its thread until the home is free.
    @Test func aSweepWaitsWhileAnotherRunMakesAndHoldsItsFolder() throws {
        let home = try openedFolder(runFoldersHome)
        var holdsTheHome = true
        defer { if holdsTheHome { release(home) } }
        #expect(flock(home, LOCK_EX) == 0)
        let making = try unheld("making")
        defer { try? FileManager.default.removeItem(at: making) }
        // This run's sweep, once it has the home, costs this pause on top of its work — many times its
        // natural time, as a loaded runner's is, yet still short enough that a sweep that skipped the
        // home would finish inside the first wait below and be caught. The wait once the home is free is
        // proven on that slowed sweep.
        sweepPause.withLock { $0 = 0.6 }
        defer { sweepPause.withLock { $0 = 0 } }
        let made = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            _ = runFolder(probeFolders)
            made.signal()
        }
        let early = made.wait(timeout: .now() + 1)
        #expect(early == .timedOut, "this run swept while another run held the home")
        #expect(exists(making), "this run swept a folder another run had made and not yet held")
        let other = try openedFolder(making)
        defer { release(other) }
        #expect(try lockedAtOnce(other, making))
        release(home)
        holdsTheHome = false
        // Wait for the folder itself: with the home free the making needs its sweep's pause, the sweeps
        // queued ahead of it and its own work, however long a loaded machine takes over them, and only a
        // sweep that never ends meets the bound. The wait happens only when the early one timed out: a
        // first wait that came back in time already took the one signal, and a second would wait out the
        // bound for nothing; a making that failed records its own issue.
        if early == .timedOut {
            let waiting = Date()
            let ended = made.wait(timeout: .now() + sweepEndsWithin)
            print("this run waited \(Date().timeIntervalSince(waiting)) s for its paused sweep once the home was free")
            fflush(stdout)
            #expect(
                ended == .success,
                "this run's sweep made no folder in the \(sweepEndsWithin) s after the home was free: it never ended")
        }
        #expect(exists(making), "this run swept a folder another run held")
    }

    @Test func aHoldWaitsForTheRunHoldingTheFolderAndLetsItGoHoweverItsBodyEnds() async throws {
        let folder = runFolder(probeFolders).appending(path: "held", directoryHint: .isDirectory)
        let other = try openedFolder(folder)
        #expect(try lockedAtOnce(other, folder))
        let entered = OSAllocatedUnfairLock(initialState: false)
        let waiting = Task { try await holding(folder) { entered.withLock { $0 = true } } }
        try await Task.sleep(for: .seconds(1))
        #expect(!entered.withLock { $0 }, "the hold went in while another run held the folder")
        release(other)
        try await waiting.value
        #expect(entered.withLock { $0 }, "the hold went in once the other run let the folder go")

        await #expect(throws: Thrown.self) { try await holding(folder) { throw Thrown() } }
        let after = try openedFolder(folder)
        defer { release(after) }
        #expect(try lockedAtOnce(after, folder), "a hold whose body threw let the folder go")
    }
}
