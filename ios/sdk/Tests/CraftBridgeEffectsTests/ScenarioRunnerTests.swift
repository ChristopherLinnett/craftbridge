// What the runner holds a scenario to, each rule pinned in its own words: a failure names where it was
// found, a folder holds exactly the names it is told, a comparison's file must hold something, a file held
// to the GPU's tolerance is held to the CPU's render, a plug-in that fails is no answer unless the scenario
// reads its failure, and a stop the export did not meet is reported as a race, never passed.

import CraftBridge
import CraftBridgeEffects
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

/// The name this suite's folders are made under and removed under together, so the making and the
/// removing cannot drift apart.
private let scenarioFoldersName = "runner"

/// Where the folders of the scenarios made here are made, for the whole suite.
private let scenarioFolders = runFolder(scenarioFoldersName)

@Suite(.serialized, Removing(name: scenarioFoldersName)) struct ScenarioRunnerTests {
    /// The scenario `calls` and `rest` make, in the corpus's own text: one step, whose calls are `calls`.
    private func scenario(calls: String, _ rest: String = "") throws -> Scenario {
        let text = """
            [[scenario]]
            id = "X"
            title = "x"
            [[scenario.step]]
            \(calls)
            \(rest)
            """
        return try #require(try Corpus(toml: text).scenarios.first)
    }

    private let info = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n"

    /// A folder with `files` in it, each by its path in the folder, made inside the suite's own folder so
    /// that the suite's ``Removing`` takes each one with the rest.
    private func folder(holding files: [String: String]) throws -> URL {
        let folder = scenarioFolders.appending(path: "files-\(UUID().uuidString)")
        for (path, text) in files {
            let file = folder.appending(path: path)
            try FileManager.default.createDirectory(
                at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            try text.write(to: file, atomically: true, encoding: .utf8)
        }
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    @Test func anExpectationThatDoesNotHoldFailsAndSaysWhereAndWhatWasFound() throws {
        let rules = try scenario(
            calls: info,
            """
            [[scenario.expect]]
            step = 0
            pointer = "/version"
            equals = "0.0.0"
            [[scenario.expect]]
            step = 0
            pointer = "/version"
            length = 3
            [[scenario.expect]]
            step = 0
            pointer = "/items"
            length = 3
            [[scenario.expect]]
            step = 0
            pointer = "/nothing"
            equals = 1
            [[scenario.expect]]
            step = 1
            pointer = ""
            equals = 1
            [[scenario.expect]]
            step = 0
            output = 4
            pointer = ""
            equals = 1
            [[scenario.expect]]
            step = 0
            pointer = "/version"
            equals = "0.24.1"
            [[scenario.expect]]
            step = 0
            pointer = "/items"
            length = 2
            [[scenario.expect]]
            step = 0
            pointer = "/n"
            equals = 90
            """
        )
        let outputs: [[JSONValue]] = [[["version": "0.24.1", "items": [1, 2], "n": 90.0]]]
        let found = rules.expects.compactMap { ScenarioRunner.failure(of: $0, in: outputs) }
        #expect(
            found == [
                #"step 0 /version: found "0.24.1", expected "0.0.0""#,
                #"step 0 /version: found "0.24.1", which holds no items"#,
                "step 0 /items: holds 2 items, expected 3",
                "step 0 /nothing: the output has no such value",
                "step 1 : the step did not run",
                "step 0 : the step printed 1 documents, none at 4",
            ],
            "\(found)"
        )
    }

    @Test func aFolderHoldsExactlyTheNamesItIsToldAndTheEnginesDotFilesDoNotCount() throws {
        let rules = try scenario(
            calls: info,
            """
            [[scenario.folder]]
            path = "."
            holds = ["out", "a.txt"]
            [[scenario.folder]]
            path = "."
            holds = ["a.txt"]
            [[scenario.folder]]
            path = "out"
            holds = ["c.txt"]
            [[scenario.folder]]
            path = "missing"
            holds = []
            """
        )
        let held = try folder(holding: ["a.txt": "a", ".config/x": "x", "out/c.txt": "c"])
        defer { try? FileManager.default.removeItem(at: held) }
        let found = rules.folders.compactMap { ScenarioRunner.failure(of: $0, in: held) }
        #expect(
            found == [#". holds ["a.txt", "out"], expected ["a.txt"]"#, "missing: cannot read the folder"],
            "\(found)"
        )
    }

    @Test func aComparisonsFileMustBeThereAndHoldSomething() throws {
        let rules = try scenario(
            calls: info,
            """
            [[scenario.compare]]
            kind = "identical"
            actual = "full.bin"
            reference = "full.bin"
            [[scenario.compare]]
            kind = "identical"
            actual = "seq"
            reference = "seq"
            [[scenario.compare]]
            kind = "identical"
            actual = "missing.bin"
            reference = "missing.bin"
            [[scenario.compare]]
            kind = "identical"
            actual = "empty.bin"
            reference = "empty.bin"
            [[scenario.compare]]
            kind = "identical"
            actual = "hollow"
            reference = "hollow"
            """
        )
        let held = try folder(holding: ["full.bin": "x", "seq/a.png": "x", "empty.bin": ""])
        defer { try? FileManager.default.removeItem(at: held) }
        try FileManager.default.createDirectory(at: held.appending(path: "hollow"), withIntermediateDirectories: true)
        let found = rules.compares.flatMap { ScenarioRunner.failures(of: $0, in: held) }
        #expect(
            found == ["missing.bin: the step wrote no such file", "empty.bin: is empty", "hollow: is empty"], "\(found)"
        )
    }

    /// A PNG file of `width` by `height` pixels in `folder`, each pixel the straight RGBA `shade` gives it.
    private func pngFile(
        _ name: String, in folder: URL, _ width: Int, _ height: Int, _ shade: (Int) -> [UInt8]
    ) throws -> URL {
        let file = folder.appending(path: name)
        try png(straight: Data((0..<width * height).flatMap(shade)), width: width, height: height).write(to: file)
        return file
    }

    @Test func anOpaquePNGIsReadAsThePixelsItHoldsAndATranslucentOneIsRefused() throws {
        let held = try folder(holding: [:])
        defer { try? FileManager.default.removeItem(at: held) }
        let shades: [[UInt8]] = [[255, 128, 64, 255], [10, 20, 30, 255], [1, 2, 3, 255], [200, 100, 50, 255]]
        let read = try straightRGBA(try pngFile("opaque.png", in: held, 2, 2) { shades[$0] })
        #expect((read.width, read.height) == (2, 2) && read.pixels == shades.flatMap { $0 })
        let translucent = try pngFile("translucent.png", in: held, 2, 2) { $0 == 0 ? [255, 128, 64, 128] : shades[$0] }
        let shown = translucent.path(percentEncoded: false)
        #expect(
            throws: RunnerFailure(
                description: "cannot read \(shown) as straight 8-bit RGBA: it holds translucent pixels, which ImageIO "
                    + "hands over premultiplied")
        ) { try straightRGBA(translucent) }
        // Sixteen bits a channel is a layout the tolerance's levels are not counted in.
        let deep = held.appending(path: "deep.png")
        let output = NSMutableData()
        let provider = try #require(CGDataProvider(data: Data(repeating: 255, count: 16) as CFData))
        let sixteen = try #require(
            CGImage(
                width: 2, height: 1, bitsPerComponent: 16, bitsPerPixel: 64, bytesPerRow: 16,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue).union(.byteOrder16Little),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent))
        let destination = try #require(
            CGImageDestinationCreateWithData(output, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, sixteen, nil)
        #expect(CGImageDestinationFinalize(destination))
        try (output as Data).write(to: deep)
        do {
            _ = try straightRGBA(deep)
            Issue.record("a PNG of sixteen bits a channel must be refused")
        } catch {
            let shown = deep.path(percentEncoded: false)
            #expect("\(error)".hasPrefix("cannot read \(shown) as straight 8-bit RGBA: 64 bits a pixel"), "\(error)")
        }
        let absent = held.appending(path: "absent.png")
        #expect(throws: RunnerFailure(description: "cannot read \(absent.path(percentEncoded: false))")) {
            try straightRGBA(absent)
        }
    }

    @Test func pixelsPassWithinTheToleranceAndNameWhatIsOverIt() throws {
        let held = try folder(holding: [:])
        defer { try? FileManager.default.removeItem(at: held) }
        let flat = { (level: UInt8) in { (_: Int) in [level, level, level, 255] } }
        let reference = try pngFile("reference.png", in: held, 10, 10, flat(100))
        let within = { (levels: UInt8, fraction: Double, cap: UInt8) in
            Tolerance(levels: levels, allowedFraction: fraction, cap: cap)
        }
        #expect(
            pixels(try pngFile("same.png", in: held, 10, 10, flat(100)), against: reference, within: within(0, 0, 0))
                .isEmpty)
        let oneOff = try pngFile("one-off.png", in: held, 10, 10, flat(101))
        #expect(pixels(oneOff, against: reference, within: within(1, 0, 1)).isEmpty)
        #expect(
            pixels(oneOff, against: reference, within: within(0, 0, 0)) == [
                "100 of 100 pixels are more than 0 levels from the reference's: 100.000% where 0.000% is allowed",
                "a pixel is 1 levels from the reference's, past the 0 allowed",
            ])
        // Three pixels in a hundred are four levels out: the fraction allows them, and the cap sets how far.
        let flips = try pngFile("flips.png", in: held, 10, 10) {
            $0 < 3 ? [104, 100, 100, 255] : [100, 100, 100, 255]
        }
        #expect(pixels(flips, against: reference, within: within(1, 0.03, 4)).isEmpty)
        #expect(
            pixels(flips, against: reference, within: within(1, 0.02, 4)) == [
                "3 of 100 pixels are more than 1 levels from the reference's: 3.000% where 2.000% is allowed"
            ])
        #expect(
            pixels(flips, against: reference, within: within(1, 0.03, 3)) == [
                "a pixel is 4 levels from the reference's, past the 3 allowed"
            ])
        #expect(
            pixels(try pngFile("wide.png", in: held, 12, 10, flat(100)), against: reference, within: within(9, 1, 9))
                == [
                    "size 12x10 is not the reference's 10x10"
                ])
    }

    @Test func aFileHeldToTheGPUsToleranceIsHeldToTheCPUsRenderOfTheSameSteps() throws {
        let tolerant = """
            [[scenario.compare]]
            kind = "pixels"
            actual = "frame.png"
            reference = "frame.png"
            tolerance = 1
            """
        let gpu = "[scenario.step.host]\ngpu = true\n"
        #expect(ScenarioRunner.heldToTheCPU(try scenario(calls: gpu + info, tolerant)))
        #expect(!ScenarioRunner.heldToTheCPU(try scenario(calls: info, tolerant)), "no step asks for the GPU")
        #expect(
            !ScenarioRunner.heldToTheCPU(
                try scenario(
                    calls: gpu + info, "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"a\"\n"
                )),
            "no file is held to the GPU's tolerance")
        let (gpuFolder, cpuFolder) = (try folder(holding: [:]), try folder(holding: [:]))
        defer { for made in [gpuFolder, cpuFolder] { try? FileManager.default.removeItem(at: made) } }
        _ = try pngFile("frame.png", in: gpuFolder, 4, 4) { _ in [102, 100, 100, 255] }
        _ = try pngFile("frame.png", in: cpuFolder, 4, 4) { _ in [100, 100, 100, 255] }
        let compare = try #require(try scenario(calls: gpu + info, tolerant).compares.first)
        #expect(
            ScenarioRunner.failures(of: compare, in: gpuFolder, onTheCPU: cpuFolder) == [
                "frame.png against the CPU's: 16 of 16 pixels are more than 1 levels from the reference's: "
                    + "100.000% where 0.000% is allowed",
                "frame.png against the CPU's: a pixel is 2 levels from the reference's, past the 1 allowed",
            ])
        #expect(ScenarioRunner.failures(of: compare, in: gpuFolder).isEmpty, "with no CPU's render, the file is there")
    }

    @Test func aScenarioOnTheGPUIsHeldToTheSameStepsMadeOnTheCPU() async throws {
        let calls = """
            [scenario.step.host]
            gpu = true
            [[scenario.step.call]]
            kind = "request"
            method = "run"
            params = { steps = [{ command = "comp.new", params = { width = 8, height = 6 } }] }
            [[scenario.step.call]]
            kind = "frame"
            format = "png"
            out = "frame.png"
            """
        let rule =
            "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"frame.png\"\nreference = \"frame.png\"\ntolerance = 1\n"
        let outcome = await ScenarioRunner(root: scenarioFolders).run(try scenario(calls: calls, rule))
        #expect(outcome.failures.isEmpty && (outcome.checks, outcome.withheld) == (2, 0), "\(outcome.failures)")
        // The core's own words for an engine started on the CPU, which the reference must be.
        #expect(outcome.rendersOn.count == 2 && outcome.rendersOn.last == "the app asked for CPU-only rendering")
        #expect(outcome.rendersOn.first != outcome.rendersOn.last, "the scenario itself ran on the GPU")
    }

    @Test func aPlugInThatFailsIsNoAnswerUnlessTheScenarioReadsItsFailure() async throws {
        let refused = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"nothing\"\n"
        let unread = await ScenarioRunner(root: scenarioFolders).run(try scenario(calls: refused))
        #expect(unread.failures.count == 1, "\(unread.failures)")
        #expect(unread.failures.first?.hasPrefix("step 0: the plug-in failed: ") == true, "\(unread.failures)")
        let read = await ScenarioRunner(root: scenarioFolders).run(
            try scenario(
                calls: refused, "[[scenario.expect]]\nstep = 0\npointer = \"/kind\"\nequals = \"Unsupported\"\n")
        )
        #expect(read.failures.isEmpty && read.steps == 1 && read.checks == 1, "\(read.failures)")
    }

    @Test func aFileTheScenarioNamesThatCannotBeReadOrWrittenIsThePlugInsFailureInItsOwnWords() async throws {
        let comp = """
            [[scenario.step.call]]
            kind = "request"
            method = "run"
            params = { steps = [{ command = "comp.new", params = { width = 8, height = 6 } }] }

            """
        let calls = [
            (
                comp + "[[scenario.step.call]]\nkind = \"frame\"\nformat = \"png\"\nout = \"nowhere/frame.png\"",
                "cannot write nowhere/frame.png: "
            ),
            (
                "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"replay\"\nfiles = { script = \"absent.jsonl\" }",
                "cannot read absent.jsonl: "
            ),
            (
                "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"replay\"\nparams = [1]\nfiles = { script = \"absent.jsonl\" }",
                "`replay`: parameters that take a file must be an object"
            ),
        ]
        for (call, words) in calls {
            let outcome = await ScenarioRunner(root: scenarioFolders).run(try scenario(calls: call))
            let said = outcome.failures.first ?? "none"
            #expect(outcome.failures.count == 1 && said.hasPrefix("step 0: the plug-in failed: \(words)"), "\(said)")
        }
    }

    @Test func aStepRendersOnTheGPUOnlyWhenTheEngineHoldsAnAdapter() {
        let device: JSONValue = [
            "gpu": "Apple M2 Max (Metal)", "project": ["renderer": ["adapter": "Apple M2 Max (Metal)"]],
        ]
        let cpu: JSONValue = ["gpu": "no adapter: none found", "project": ["renderer": ["adapter": nil]]]
        #expect(ScenarioRunner.failure(of: device, onTheGPU: true) == nil)
        #expect(ScenarioRunner.failure(of: cpu, onTheGPU: false) == nil)
        #expect(
            ScenarioRunner.failure(of: cpu, onTheGPU: true)
                == RunnerFailure(
                    description: #"the step needs the GPU, and the engine renders on the CPU: "no adapter: none found""#
                )
        )
        #expect(
            ScenarioRunner.failure(of: [:], onTheGPU: true)
                == RunnerFailure(
                    description: "the step needs the GPU, and the engine renders on the CPU: it says nothing of why")
        )
        #expect(
            ScenarioRunner.failure(of: device, onTheGPU: false)
                == RunnerFailure(description: #"the step's render on the CPU is on the GPU, "Apple M2 Max (Metal)""#)
        )
    }

    @Test func aStopTheRenderQueueDoesNotSayTheExportMetIsAnIssueAndNeverAPass() throws {
        let listening = Listening(stop: try stopping(at: "0.5"))
        let at = Report(done: 160, total: 300)
        #expect(listening.judge(stoppedAt: at, statuses: ["Unqueued", "UserStopped"]) == nil)
        #expect(
            listening.judge(stoppedAt: at, statuses: ["Unqueued", "Done"])
                == #"the stop came at 160 of 300 frames, and the render queue says ["Unqueued","Done"], "#
                + #"with no export "UserStopped": every frame was rendered before the stop reached the export, "#
                + "so the export won the race, which the SDK has no way to hold"
        )
        #expect(listening.judge(stoppedAt: at, statuses: []) != nil, "a queue with no item says no export met it")
        #expect(
            listening.judge(stoppedAt: nil, statuses: ["UserStopped"])
                == "the call answered before its export reached 0.5 of its frames, so no stop reached it: "
                + "the export won the race, which the SDK has no way to hold"
        )
        #expect(
            Listening(stop: try stopping(at: "1")).judge(stoppedAt: nil, statuses: [])
                == "the call answered before its export reached 1 of its frames, so no stop reached it: "
                + "the export won the race, which the SDK has no way to hold",
            "the fraction is named as the JSON it is"
        )
        let alone = Listening(stop: try stopping(at: nil))
        #expect(
            alone.judge(stoppedAt: nil, statuses: []) == nil, "a call with no fraction to stop at is judged by nothing")
    }

    /// A call the corpus makes that reports progress starts through the engine's one job helper, so the
    /// background controller keeps it out of suspension as it does an export an app makes.
    @Test @MainActor func aCallTheCorpusListensToRunsUnderABackgroundTask() async throws {
        let host = FakeHost()
        let engine = EffectsEngine(engine: FakeEngine(), host: host)
        _ = try await Listening(stop: try stopping(at: nil)).run("export", [:], on: engine)
        #expect(host.begun == [1])
        #expect(await eventually { host.ended == [1] })
    }

    /// A stop the listener heard before it held the job is asked of the job as it is handed over, and one it
    /// heard after is asked at once: neither is lost to the order the engine's report and the caller meet in.
    @Test(arguments: [true, false]) func aStopIsAskedOfTheJobWhicheverComesFirst(heardFirst: Bool) async throws {
        let (stream, finish) = AsyncStream<Void>.makeStream()
        let fake = FakeEngine()
        fake.answer = { _, _ in
            for await _ in stream { break }
            return "null"
        }
        let job = try ExportJob(engine: fake, method: "export", params: [:])
        let hearing = Hearing(stop: try stopping(at: "0.5"))
        if heardFirst { hearing.hear(done: 160, total: 300) }
        #expect(!job.isCancelled)
        hearing.take(job)
        if !heardFirst {
            #expect(!job.isCancelled, "nothing has been heard to stop the job at")
            hearing.hear(done: 160, total: 300)
        }
        #expect(job.isCancelled)
        #expect(hearing.stoppedAt == Report(done: 160, total: 300))
        finish.yield()
        _ = try? await job.result
    }

    @Test func theRenderQueueIsAskedForTheStatusOfEachItem() async throws {
        let asked = Requests()
        let listed = FakeEngine(recording: asked)
        listed.answer = { _, _ in
            #"{"items": [{"status": "Unqueued"}, {"status": "UserStopped"}, {"status": {"Failed": "x"}}]}"#
        }
        let statuses = try await Listening.statuses(on: EffectsEngine(engine: listed))
        #expect(statuses == ["Unqueued", "UserStopped", ["Failed": "x"]])
        let (method, params) = (await asked.method(), await asked.params())
        #expect(method == "exec" && params == ["command": "renderQueue.list", "params": [:]])
        let empty = FakeEngine()
        empty.answer = { _, _ in "{}" }
        await #expect(throws: RunnerFailure(description: "the render queue lists no items: {}")) {
            try await Listening.statuses(on: EffectsEngine(engine: empty))
        }
    }

    private func stopping(at fraction: String?) throws -> Stop {
        let progress = fraction.map { "{ cancel_at = \($0) }" } ?? "{}"
        let call = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"export\"\nprogress = \(progress)\n"
        guard case .request(_, _, _, let stop?)? = try scenario(calls: call).steps.first?.calls.first else {
            throw RunnerFailure(description: "no stop in \(progress)")
        }
        return stop
    }
}
