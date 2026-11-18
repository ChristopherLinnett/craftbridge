// The app tools/ios-device-test.sh wraps a Swift package's test bundle in, to run its Swift Testing suite on the
// acceptance iPhone. A package's test target has no host app, and `xcodebuild test` refuses one on a device, so
// the runner makes this one: it is compiled with `swiftc` there, with no Xcode project, and the runner signs and
// installs it like the Rust test apps. It has two roles, by its bundle id:
//
//   host   (any id) loads the test bundles in its PlugIns folder and runs them through Swift Testing's entry point
//          for tools, `swt_abiv0_getEntryPoint`, whose JSON records it turns into the lines the runner reads
//          from a Rust test binary (`running N tests`, `test <name> ... ok`, `test result: ...`).
//   cover  (an id ending `.cover`) is a second app with nothing in it, which the runner brings to the front to
//          send the host to the background, and then brings the host back.
//
// The host reads its arguments as Swift Testing's own command line does (`--filter`, `--skip`), since it hands
// the entry point no configuration of its own.

import Darwin
import Foundation
import UIKit

/// The entry point's type, as Swift Testing documents it for tools that do not link it (`ABI.v0.EntryPoint`).
private typealias EntryPoint =
    @convention(thin) @Sendable (
        _ configurationJSON: UnsafeRawBufferPointer?,
        _ recordHandler: @escaping @Sendable (_ recordJSON: UnsafeRawBufferPointer) -> Void
    ) async throws -> Bool

/// What the run's records say, one test at a time, kept for the lines the runner reads. A result the host
/// cannot read fails the run, never passes it: a record that is no JSON object, an issue no test of the run
/// ended with, and a test the run planned that nothing ended or skipped are each a failure, named.
private final class Report: @unchecked Sendable {
    /// What a failure no test function carries is reported under: an issue recorded outside every test, or a
    /// record the host could not read.
    private static let outside = "(outside every test)"

    private let lock = NSLock()
    /// The test functions the run plans, in the order the records name them.
    private var planned: [String] = []
    private var failures: [String: [String]] = [:]
    private var cases: [String: Int] = [:]
    private var skipped: Set<String> = []
    private var ended: Set<String> = []
    private var running: [String: Date] = [:]
    private(set) var passed = 0
    private(set) var failed = 0
    private(set) var ignored = 0

    /// Takes one record, a JSON document the entry point streams.
    func take(_ json: Data) {
        lock.lock()
        defer { lock.unlock() }
        guard let record = (try? JSONSerialization.jsonObject(with: json)) as? [String: Any],
            let payload = record["payload"] as? [String: Any]
        else {
            let text = String(decoding: json.prefix(200), as: UTF8.self)
            failures[Self.outside, default: []].append("a record the host cannot read: \(text)")
            return
        }
        // A record of a kind the schema does not name is skipped, as the schema asks of a reader.
        switch record["kind"] as? String {
        case "test": test(payload)
        case "event": event(payload)
        default: break
        }
    }

    private func test(_ payload: [String: Any]) {
        guard payload["kind"] as? String == "function", let id = payload["id"] as? String else { return }
        planned.append(id)
    }

    private func event(_ payload: [String: Any]) {
        let id = (payload["testID"] as? String).flatMap { planned.contains($0) ? $0 : nil }
        let messages = (payload["messages"] as? [[String: Any]]) ?? []
        switch payload["kind"] as? String {
        case "runStarted":
            // Every test record comes before the run starts, a run that plans none included.
            say("running \(planned.count) test\(planned.count == 1 ? "" : "s")")
        case "testStarted":
            if let id { running[id] = Date() }
        case "testCaseStarted":
            if let id { cases[id, default: 0] += 1 }
        case "testSkipped", "testCancelled":
            // A skipped test never starts, so no end follows; a cancelled one has started, and ends.
            if let id {
                skipped.insert(id)
                if running[id] == nil { finish(id) }
            }
        case "issueRecorded":
            // A known issue and a warning are no failure; every other issue is, wherever it was recorded: one
            // on a suite, or outside every test, is reported when the run ends.
            let issue = payload["issue"] as? [String: Any]
            let isFailure = messages.contains { $0["symbol"] as? String == "fail" }
            if isFailure, issue?["isKnown"] as? Bool != true {
                let key = id ?? (payload["testID"] as? String) ?? Self.outside
                let text = messages.compactMap { $0["text"] as? String }.joined(separator: " / ")
                failures[key, default: []].append(text)
            }
        case "testEnded":
            if let id { finish(id) }
        default: break
        }
    }

    private func finish(_ id: String) {
        guard ended.insert(id).inserted else { return }
        running[id] = nil
        if let count = cases[id], count > 1 { say("cases \(id): \(count)") }
        if let issues = failures.removeValue(forKey: id) {
            fail(id, issues)
        } else if skipped.contains(id) {
            ignored += 1
            say("test \(id) ... ignored")
        } else {
            passed += 1
            say("test \(id) ... ok")
        }
    }

    private func fail(_ id: String, _ issues: [String]) {
        failed += 1
        for issue in issues { say("failure \(id): \(issue)") }
        say("test \(id) ... FAILED")
    }

    /// Fails the test that has run longer than `limit` seconds without ending, if one has, and then returns
    /// the run's last line, the run having begun at `began`: a hung test would otherwise hold the run to the
    /// launch's limit, naming nothing.
    func abandonOverdue(after limit: TimeInterval, began: Date) -> String? {
        lock.lock()
        defer { lock.unlock() }
        guard let (id, _) = running.first(where: { Date().timeIntervalSince($0.value) > limit }) else { return nil }
        failures[id, default: []].append("the test had not ended \(Int(limit)) seconds after it started")
        finish(id)
        reportUnended()
        return summary(seconds: Date().timeIntervalSince(began))
    }

    /// Reports what no test's end reported once the entry point has returned, `succeeded` its own answer,
    /// and returns the run's last line, in the shape of a Rust harness's. The entry point fails a run on any
    /// failing issue it records, so a run it fails that the records show no failure of fails here too.
    func end(succeeded: Bool, seconds: Double) -> String {
        lock.lock()
        defer { lock.unlock() }
        for id in planned where !ended.contains(id) && failures[id] == nil {
            failures[id] = ["the run ended with no result for the test"]
        }
        reportUnended()
        if !succeeded, failed == 0 {
            fail(Self.outside, ["Swift Testing answered that the run failed, and no record the host read says why"])
        }
        return summary(seconds: seconds)
    }

    /// Reports every failure no test's end reported: one on a suite or outside every test, one the plan
    /// recorded for a test in place of running it, and one recorded after its test ended.
    private func reportUnended() {
        for (id, issues) in failures.sorted(by: { $0.key < $1.key }) {
            ended.insert(id)
            fail(id, issues)
        }
        failures = [:]
    }

    /// The run's last line, in the shape of a Rust harness's.
    private func summary(seconds: Double) -> String {
        let outcome = failed == 0 ? "ok" : "FAILED"
        return "test result: \(outcome). \(passed) passed; \(failed) failed; \(ignored) ignored; 0 measured; "
            + "0 filtered out; finished in \(String(format: "%.2f", seconds))s"
    }
}

/// One line on the console, the way the runner reads it.
private func say(_ line: String) {
    print(line)
    fflush(stdout)
}

/// Loads every test bundle in the app's PlugIns folder, and returns the entry point Swift Testing documents.
private func entryPoint() -> EntryPoint? {
    let plugIns = Bundle.main.builtInPlugInsURL
    let bundles =
        (try? FileManager.default.contentsOfDirectory(
            at: plugIns ?? Bundle.main.bundleURL, includingPropertiesForKeys: nil)) ?? []
    for bundle in bundles where bundle.pathExtension == "xctest" {
        do {
            try Bundle(url: bundle)?.loadAndReturnError()
        } catch {
            say("ios-test-host: cannot load \(bundle.lastPathComponent): \(error)")
            return nil
        }
    }
    guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "swt_abiv0_getEntryPoint") else {
        say("ios-test-host: no swt_abiv0_getEntryPoint in the loaded images: \(String(cString: dlerror()))")
        return nil
    }
    typealias Getter = @convention(c) () -> UnsafeRawPointer
    return unsafeBitCast(unsafeBitCast(symbol, to: Getter.self)(), to: EntryPoint.self)
}

/// Runs the suite and returns the process's exit status.
private func runSuite() async -> Int32 {
    let report = Report()
    let began = Date()
    // A thread of its own: a hung test may hold every thread the concurrency runtime has.
    if let given = ProcessInfo.processInfo.environment["CRAFTBRIDGE_RUNNER_TEST_LIMIT"] {
        guard let limit = TimeInterval(given), limit > 0 else {
            say("ios-test-host: CRAFTBRIDGE_RUNNER_TEST_LIMIT is \(given), which is no number of seconds")
            return 2
        }
        Thread.detachNewThread {
            while true {
                Thread.sleep(forTimeInterval: 5)
                guard let last = report.abandonOverdue(after: limit, began: began) else { continue }
                say(last)
                exit(1)
            }
        }
    }
    guard let run = entryPoint() else { return 2 }
    let succeeded: Bool
    do {
        succeeded = try await run(nil) { report.take(Data($0)) }
    } catch {
        say("ios-test-host: the run did not start: \(error)")
        return 2
    }
    say(report.end(succeeded: succeeded, seconds: Date().timeIntervalSince(began)))
    return report.failed == 0 ? 0 : 1
}

private final class Delegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        let isCover = Bundle.main.bundleIdentifier?.hasSuffix(".cover") == true
        let screen = UIViewController()
        screen.view.backgroundColor = isCover ? .systemRed : .systemBlue
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = screen
        window?.makeKeyAndVisible()
        // The phone turns its screen off and locks once its Auto-Lock time passes without a touch, and suspends
        // the app with it; a run is hands-off, and a suspended app's tests hang until the launch limit.
        application.isIdleTimerDisabled = true
        if !isCover {
            Task.detached { exit(await runSuite()) }
        }
        return true
    }
}

setvbuf(stdout, nil, _IOLBF, 0)
_ = UIApplicationMain(CommandLine.argc, CommandLine.unsafeArgv, nil, NSStringFromClass(Delegate.self))
