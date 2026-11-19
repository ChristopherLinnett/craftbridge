// What a test that runs only when asked for by name reads of the run's arguments.

import Testing

@Suite struct AskedByNameTests {
    @Test func aFilterThatIsTheNameAsksForIt() {
        #expect(askedByName("RealAppPhotosTests", in: ["host", "--no-parallel", "--filter", "RealAppPhotosTests"]))
        #expect(askedByName("RealAppPhotosTests", in: ["--filter", "RealApp", "--filter", "RealAppPhotosTests"]))
    }

    @Test func aRunWithNoFilterOrAnotherOneDoesNotAskForIt() {
        #expect(!askedByName("RealAppPhotosTests", in: ["host", "--no-parallel"]))
        #expect(!askedByName("RealAppPhotosTests", in: ["--filter", "MovieTracksTests"]))
        #expect(!askedByName("RealAppPhotosTests", in: ["--filter"]))
        #expect(!askedByName("RealAppPhotosTests", in: ["--skip", "RealAppPhotosTests"]))
        #expect(!askedByName("RealAppPhotosTests", in: ["RealAppPhotosTests"]))
    }

    @Test func aFilterThatReachesTheSuiteWithoutBeingItsNameDoesNotAskForIt() {
        // The filters other rounds run the device runner with, and every other way a filter takes the suite in.
        for filter in [
            "RealApp", "RealAppPhotos", "Photos", "RealAppPhotosTests|RealAppSpineTests", ".*RealAppPhotosTests.*",
            "RealAppPhotosTests/theRunningExamplesMovieWithSoundIsSavedToPhotosForAPersonToWatch()",
            "CraftBridgeEffectsTests.RealAppPhotosTests", "CraftBridgeEffectsTests", "realappphotostests",
            "RealAppPhotosTests ",
        ] {
            #expect(!askedByName("RealAppPhotosTests", in: ["--filter", filter]), "--filter \(filter) asks for it")
        }
    }

    @Test func theTraitLeavesTheTestOutOfARunThatIsNotTheRunnersEvenWhenItAsksByName() async throws {
        // Nothing in this run is the device runner's, which the trait and the test's own guard need before they
        // look at the arguments.
        #expect(!underTheDeviceRunner, "this run is not the device runner's")
        #expect(!deviceRunAsks(for: "AskedByNameTests"), "a run with no runner asks for a test by name")
        let trait: ConditionTrait = .needsTheDeviceRunner(askedFor: "AskedByNameTests")
        #expect(try await trait.evaluate() == false, "a run with no runner left the test in")
    }
}
