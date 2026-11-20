// The baseline file's own rules: the definition the runs read, the line they print, and what a device record
// must hold, which the release script relies on when it reads the record.

import CraftBridge
import Foundation
import Testing

/// A baseline of four figures, two of which only one platform measures, with `devices` after it. Its repeats' word
/// and decimals are not the file's own, so a line that wrote either itself rather than reading them would show it.
private func baseline(_ devices: String = "") -> String {
    """
    line = "craftbridge-baseline"
    repeats_word = "repeats_ms"
    repeats_decimals = 3
    flag_slower_than_percent = 10
    [comparison]
    runs = 5
    [running_example]
    width = 1080
    height = 1080
    seconds = 6
    frame_rate = 30
    frames = 180
    format = "h264"
    preview_at_seconds = 2
    preview_repeats = 9
    [[figure]]
    name = "first_preview_ms"
    decimals = 1
    flagged = true
    unheld_on = ["android"]
    [[figure]]
    name = "full_export_ms"
    decimals = 1
    flagged = true
    [[figure]]
    name = "thermal_state"
    decimals = 0
    flagged = false
    platforms = ["ios"]
    [[figure]]
    name = "cpu_cap_mhz"
    decimals = 0
    flagged = false
    platforms = ["android"]
    \(devices)
    """
}

/// The spread of five runs, 1 to 5, as a record writes it.
private let spread = "median = 3.0\nmin = 1.0\nmax = 5.0\nvalues = [3.0, 1.0, 5.0, 2.0, 4.0]"

/// A device record of five runs, on `platform`, with `extra` lines after its keys, and the figures that platform measures.
private func device(
    id: String = "phone", platform: String = "ios", runs: Int = 5, spread spreadOfRuns: String = spread,
    extra: String = ""
) -> String {
    let conditions =
        platform == "ios"
        ? "[device.thermal_state]\nmedian = 0.0\nmin = 0.0\nmax = 0.0\nvalues = [0.0, 0.0, 0.0, 0.0, 0.0]\n"
        : "[device.cpu_cap_mhz]\n\(spread)\n"
    return """
        [[device]]
        id = "\(id)"
        platform = "\(platform)"
        device = "iPhone"
        os = "iOS 26"
        build = "release"
        conditions = "quiet"
        recorded = "2026-11-19"
        runs = \(runs)
        \(extra)
        [device.first_preview_ms]
        \(spreadOfRuns)
        [device.full_export_ms]
        \(spreadOfRuns)
        \(conditions)
        """
}

@Suite struct BaselineTests {
    @Test func theFileDefinesTheRunningExampleAndItsFiguresInTheOrderTheLinePrintsThem() throws {
        let baseline = try Baseline.current()
        #expect(baseline.line == "craftbridge-baseline")
        #expect(baseline.repeatsWord == "preview_repeats_ms" && baseline.repeatsDecimals == 2)
        #expect(
            baseline.figures
                == [
                    .init(name: "first_preview_ms", decimals: 1, flagged: true, unheldOn: ["android"]),
                    .init(name: "gpu_start_ms", decimals: 1, flagged: false),
                    .init(name: "first_path_ms", decimals: 1, flagged: true, unheldOn: ["android"]),
                    .init(name: "preview_path_ms", decimals: 1, flagged: true, unheldOn: ["android"]),
                    .init(name: "full_export_ms", decimals: 1, flagged: true),
                    .init(name: "peak_memory_bytes", decimals: 0, flagged: true),
                    .init(name: "export_threads", decimals: 0, flagged: false),
                    .init(name: "thermal_state", decimals: 0, flagged: false, platforms: ["ios"]),
                    .init(name: "cpu_cap_mhz", decimals: 0, flagged: false, platforms: ["android"]),
                    .init(name: "battery_c", decimals: 1, flagged: false, platforms: ["android"]),
                ],
            "the costs a release is flagged on, the GPU start that most of the first preview is, and the conditions")
        #expect(baseline.flagSlowerThanPercent == 10)
        #expect(baseline.comparisonRuns == 10)
        #expect(
            baseline.example
                == Baseline.Example(
                    width: 1080, height: 1080, seconds: 6, frameRate: 30, frames: 180, format: "h264",
                    previewAt: 2, previewRepeats: 9),
            "the running example of plan §5")
        #expect(baseline.example.frames == baseline.example.seconds * baseline.example.frameRate)
        #expect(
            baseline.example.seconds == cardSeconds && baseline.example.frameRate == cardFrameRate,
            "the card the baseline measures is the running example the other device tests build")
    }

    @Test func aFigureIsHeldOnEveryPlatformThatMeasuresItUnlessTheBaselineNamesOneWhereItIsNot() throws {
        let figures = try Baseline(toml: baseline()).figures
        let first = try #require(figures.first { $0.name == "first_preview_ms" })
        #expect(first.isHeld(on: "ios") && !first.isHeld(on: "android"), "reported where its own noise is too wide")
        let export = try #require(figures.first { $0.name == "full_export_ms" })
        #expect(export.isHeld(on: "ios") && export.isHeld(on: "android"))
        let thermal = try #require(figures.first { $0.name == "thermal_state" })
        #expect(!thermal.isHeld(on: "ios"), "a condition is never flagged")
        let ios = Baseline.Figure(name: "first_frame_ms", decimals: 1, flagged: true, platforms: ["ios"])
        #expect(ios.isHeld(on: "ios") && !ios.isHeld(on: "android"), "a platform that does not measure it")
    }

    @Test func aFigureUnheldWhereItIsNotMeasuredOrNeverFlaggedIsRefused() {
        let ios = "platforms = [\"ios\"]"
        func unheld(on platform: String) -> String {
            baseline().replacingOccurrences(of: ios, with: "\(ios)\nunheld_on = [\"\(platform)\"]")
        }
        let elsewhere = "the figure `thermal_state` is unheld on `android`, which does not measure it"
        #expect(throws: CraftError.InvalidParams(detail: elsewhere)) { try Baseline(toml: unheld(on: "android")) }
        let never = "the figure `thermal_state` is unheld on a platform and never flagged"
        #expect(throws: CraftError.InvalidParams(detail: never)) { try Baseline(toml: unheld(on: "ios")) }
    }

    @Test func aFigureUnheldEverywhereOrWhosePlatformsAreSpeltOtherwiseIsRefused() {
        func spelt(_ text: String, for listed: String = "unheld_on = [\"android\"]") -> String {
            baseline().replacingOccurrences(of: listed, with: text)
        }
        let everywhere =
            "the figure `first_preview_ms` is unheld everywhere it is measured, which `flagged = false` says"
        #expect(throws: CraftError.InvalidParams(detail: everywhere)) {
            try Baseline(toml: spelt("unheld_on = [\"ios\", \"android\"]"))
        }
        let unknown = "the figure `first_preview_ms` names the platform `andriod`, which is neither ios nor android"
        #expect(throws: CraftError.InvalidParams(detail: unknown)) {
            try Baseline(toml: spelt("unheld_on = [\"andriod\"]"))
        }
        let unlisted = "the figure `first_preview_ms`'s `unheld_on` is not a list of platforms"
        #expect(throws: CraftError.InvalidParams(detail: unlisted)) {
            try Baseline(toml: spelt("unheld_on = \"android\""))
        }
        let misspelt = "the figure `first_preview_ms` holds the key `unheld_in`, which no figure has"
        #expect(throws: CraftError.InvalidParams(detail: misspelt)) {
            try Baseline(toml: spelt("unheld_in = [\"android\"]"))
        }
        let measured = "the figure `thermal_state`'s `platforms` is not a list of platforms"
        #expect(throws: CraftError.InvalidParams(detail: measured)) {
            try Baseline(toml: spelt("platforms = \"ios\"", for: "platforms = [\"ios\"]"))
        }
    }

    @Test func aRunPrintsOneLineOfTheFiguresItsPlatformMeasuresByNameInTheDefinedOrderThenEveryWarmRepeat() throws {
        let baseline = try Baseline(toml: baseline())
        let repeats = [30, 1.25, 20.0004, 22, 21.5, 40, 25, 24, 23]
        #expect(
            try baseline.line(
                for: ["thermal_state": 1, "first_preview_ms": 1234.56, "full_export_ms": 6789], repeats: repeats)
                == "craftbridge-baseline first_preview_ms=1234.6 full_export_ms=6789.0 thermal_state=1 "
                + "repeats_ms=30.000,1.250,20.000,22.000,21.500,40.000,25.000,24.000,23.000")
        let missing = CraftError.InvalidParams(
            detail:
                "the run measured [\"first_preview_ms\"], and the baseline names [\"first_preview_ms\", \"full_export_ms\", \"thermal_state\"]"
        )
        #expect(throws: missing) { try baseline.line(for: ["first_preview_ms": 1], repeats: repeats) }
        #expect(throws: CraftError.self, "a figure another platform measures is no figure of this one") {
            try baseline.line(
                for: ["first_preview_ms": 1, "full_export_ms": 1, "thermal_state": 1, "cpu_cap_mhz": 1],
                repeats: repeats)
        }
        // One repeat fewer and one more than the example makes: the nearest a wrong count comes, which a looser rule
        // would let through.
        let run: [String: Double] = ["first_preview_ms": 1, "full_export_ms": 1, "thermal_state": 1]
        for (made, refused) in [
            (8, "the run made 8 warm previews, and the running example makes 9"),
            (10, "the run made 10 warm previews, and the running example makes 9"),
        ] {
            #expect(throws: CraftError.InvalidParams(detail: refused)) {
                try baseline.line(for: run, repeats: Array(repeating: 20, count: made))
            }
        }
    }

    @Test func theMedianIsTheMiddleValueOrTheMeanOfTheTwoInTheMiddle() {
        #expect(Baseline.median(of: [5, 1, 3]) == 3)
        #expect(Baseline.median(of: [4, 1, 3, 2]) == 2.5)
        #expect(Baseline.median(of: [7]) == 7)
    }

    @Test func theRecordTheFileHoldsIsComplete() throws {
        let problems = try Baseline.current().recordProblems()
        #expect(problems == [], "the record the file holds has problems: \(problems)")
    }

    @Test func aRecordThatIsCompleteHasNoProblem() throws {
        #expect(
            try Baseline(toml: baseline(device() + device(id: "another", platform: "android"))).recordProblems() == [])
        let alike = "median = 2.0\nmin = 2.0\nmax = 2.0\nvalues = [2.0, 2.0, 2.0, 2.0, 2.0]"
        #expect(
            try Baseline(toml: baseline(device(spread: alike))).recordProblems() == [],
            "runs that all measured alike have one figure for their median and both ends")
    }

    @Test func eachWayARecordCanBeIncompleteIsNamed() throws {
        func problems(_ devices: String) throws -> [String] { try Baseline(toml: baseline(devices)).recordProblems() }
        #expect(
            try problems(device(runs: 4)) == [
                "phone: 4 runs, fewer than the 5 a baseline rests on",
                "phone: `first_preview_ms` holds 5 values for 4 runs",
                "phone: `full_export_ms` holds 5 values for 4 runs",
                "phone: `thermal_state` holds 5 values for 4 runs",
            ], "a run missing from a record, or a record kept for runs it lost, is named")
        #expect(try problems(device() + device()) == ["phone: a second record has this id"])
        #expect(try problems(device(extra: "color = \"red\"")) == ["phone: the key `color` is not read"])
        #expect(
            try problems(device().replacingOccurrences(of: "os = \"iOS 26\"\n", with: "")) == ["phone: no `os`"])
        #expect(
            try problems(device().replacingOccurrences(of: "conditions = \"quiet\"\n", with: ""))
                == ["phone: no `conditions`"],
            "a figure is compared only with one made under the same conditions, so a record says them")
        #expect(
            try problems(device().replacingOccurrences(of: "platform = \"ios\"", with: "platform = \"web\""))
                == [
                    "phone: the platform `web` is neither ios nor android",
                    "phone: the key `first_preview_ms` is not read",
                    "phone: the key `full_export_ms` is not read",
                    "phone: the key `thermal_state` is not read",
                ])
    }

    @Test func aRecordHoldsTheFiguresItsPlatformMeasuresAndNoOtherPlatformsOnes() throws {
        func problems(_ devices: String) throws -> [String] { try Baseline(toml: baseline(devices)).recordProblems() }
        #expect(
            try problems(device().replacingOccurrences(of: "[device.thermal_state]", with: "[device.cpu_cap_mhz]"))
                == [
                    "phone: the key `cpu_cap_mhz` is not read",
                    "phone: `thermal_state` is not a table of a median, a min, a max and the values of its runs",
                ], "a record made on a phone holds the conditions that phone reads: a record missing its own is named")
        #expect(
            try problems(
                device().replacingOccurrences(
                    of: "[device.full_export_ms]\n\(spread)\n", with: ""))
                == ["phone: `full_export_ms` is not a table of a median, a min, a max and the values of its runs"])
    }

    @Test func aFigureHoldsTheValuesOfItsRunsAndTheMedianAndRangeTheyGiveAndNothingElse() throws {
        func problems(_ devices: String) throws -> [String] { try Baseline(toml: baseline(devices)).recordProblems() }
        let extra = device(spread: spread + "\nmean = 2.0")
        #expect(
            try problems(extra) == [
                "phone: `first_preview_ms` is not a table of a median, a min, a max and the values of its runs",
                "phone: `full_export_ms` is not a table of a median, a min, a max and the values of its runs",
            ])
        let moved = device(
            spread: "median = 4.0\nmin = 1.0\nmax = 5.0\nvalues = [3.0, 1.0, 5.0, 2.0, 4.0]")
        #expect(
            try problems(moved) == [
                "phone: `first_preview_ms` says median, min and max [4.0, 1.0, 5.0], and its values give [3.0, 1.0, 5.0]",
                "phone: `full_export_ms` says median, min and max [4.0, 1.0, 5.0], and its values give [3.0, 1.0, 5.0]",
            ], "a median that is not the values' is a record that lost or changed a run")
        let lost = device(spread: "median = 3.0\nmin = 1.0\nmax = 5.0\nvalues = [3.0, 1.0, 5.0, 4.0]")
        #expect(
            try problems(lost) == [
                "phone: `first_preview_ms` holds 4 values for 5 runs",
                "phone: `full_export_ms` holds 4 values for 5 runs",
            ])
        let zero = device(spread: "median = 3.0\nmin = 0.0\nmax = 5.0\nvalues = [3.0, 0.0, 5.0, 2.0, 4.0]")
        #expect(
            try problems(zero) == [
                "phone: `first_preview_ms` has a value that is no measurement: 0.0",
                "phone: `full_export_ms` has a value that is no measurement: 0.0",
            ], "a cost of zero is no measurement; a condition of zero, nominal heat, is one")
    }

    @Test func aFigureThatDoesNotSayWhetherItIsFlaggedIsRefused() throws {
        #expect(throws: CraftError.InvalidParams(detail: "the baseline has no true or false `flagged`")) {
            try Baseline(toml: baseline().replacingOccurrences(of: "flagged = false\n", with: ""))
        }
        #expect(throws: CraftError.InvalidParams(detail: "the baseline has no true or false `flagged`")) {
            try Baseline(toml: baseline().replacingOccurrences(of: "flagged = false", with: "flagged = 0"))
        }
        #expect(try Baseline(toml: baseline()).figures.map(\.flagged) == [true, true, false, false])
    }

    @Test func theExportRanOnThePoolItKeepsAndOtherwiseOnTheCallersThreadsOneForEachProcessor() throws {
        #expect(
            try exportThreads() == Double(ProcessInfo.processInfo.activeProcessorCount),
            "no export keeps a pool here, so an export runs on the caller's threads")
        let named = DispatchSemaphore(value: 0)
        let finished = DispatchSemaphore(value: 0)
        let pool = Thread {
            Thread.current.name = "craftbridge-export-0"
            named.signal()
            finished.wait()
        }
        pool.start()
        named.wait()
        defer { finished.signal() }
        #expect(try exportThreads() == 1, "a pool of one thread, named as the exporter names its pool's")
    }

    @Test func aFigureMeasuredOnAPlatformTheBaselineDoesNotKnowIsRefused() {
        #expect(
            throws: CraftError.InvalidParams(
                detail: "the figure `cpu_cap_mhz` names the platform `andriod`, which is neither ios nor android")
        ) {
            try Baseline(
                toml: baseline().replacingOccurrences(
                    of: "platforms = [\"android\"]", with: "platforms = [\"andriod\"]"))
        }
    }

    @Test func aDefinitionThatNamesNoFigureOrAnUnreadTableIsRefused() {
        let text = baseline()
        let figureless = String(text[..<(text.range(of: "[[figure]]")?.lowerBound ?? text.startIndex)])
        #expect(throws: CraftError.InvalidParams(detail: "the baseline defines no figure")) {
            try Baseline(toml: figureless)
        }
        #expect(throws: CraftError.InvalidParams(detail: "the baseline holds the table `other`, which no runner reads"))
        {
            try Baseline(toml: baseline("[other]\nx = 1"))
        }
        let dated = baseline(
            device().replacingOccurrences(of: "recorded = \"2026-11-19\"", with: "recorded = 2026-11-19"))
        let refused = #expect(throws: CraftError.self) { try Baseline(toml: dated) }
        #expect(
            String(describing: refused).contains("the baseline holds a TOML value JSON has none for"),
            "a TOML date is refused in the baseline's own name: \(String(describing: refused))")
    }
}
