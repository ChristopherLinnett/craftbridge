// The conformance corpus on the phone (plan §28, QA-2): every scenario of `conformance/effects/corpus.toml`
// the plug-in runs, made through the SDK on the engine it starts, one test case each, named by the
// scenario's id. T24 gains the case the desktop cannot prove, which iOS alone decides: the plug-in folder
// is the one inside the app's bundle.

import CraftBridge
import CraftBridgeEffects
import CraftBridgeFFI
import Foundation
import TOMLDecoder
import Testing

extension Corpus {
    /// The scenarios the plug-in runs, which is every one the oracle does not run alone. Empty when the
    /// file does not parse, which ``ConformanceTests/theCorpusParsesAndTheSwiftRunTakesEveryScenarioThePlugInRuns()``
    /// reports.
    static let pluginScenarios: [Scenario] = ((try? Corpus(file: file))?.scenarios ?? []).filter(\.sides.runsPlugin)
}

extension Scenario: CustomTestStringConvertible {
    /// The id and the title, which the test log names the case by.
    var testDescription: String { "\(id): \(title)" }
}

/// The name this suite's folders are made under and removed under together, so the making and the
/// removing cannot drift apart.
private let scenarioFoldersName = "conformance"

/// Where the corpus scenarios' folders are made, for the whole suite.
private let scenarioFolders = runFolder(scenarioFoldersName)

@Suite(.serialized, .timeLimit(.minutes(10)), Removing(name: scenarioFoldersName)) struct ConformanceTests {
    @Test func theCorpusParsesAndTheSwiftRunTakesEveryScenarioThePlugInRuns() throws {
        _ = try Corpus(file: Corpus.file)
        // The scenarios the plug-in runs are counted from the document itself, not through the model or
        // its filter, so a scenario either of them loses is named here.
        let document = try [String: Any](TOMLTable(source: String(contentsOf: Corpus.file, encoding: .utf8)))
        let tables = (document["scenario"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }
        let corpus = tables.filter { $0["sides"] as? String != "oracle" }.compactMap { $0["id"] as? String }
        let run = Corpus.pluginScenarios.map(\.id)
        #expect(!corpus.isEmpty, "the corpus holds scenarios the plug-in runs")
        let (left, added) = (corpus.filter { !run.contains($0) }, run.filter { !corpus.contains($0) })
        #expect(run == corpus, "the Swift run leaves out \(left) and adds \(added), in the order \(run)")
        print("the Swift run takes \(run.count) scenarios, of the corpus's \(corpus.count) that the plug-in runs")
    }

    @Test(arguments: Corpus.pluginScenarios)
    func theScenarioPassesThroughTheSwiftAPI(_ scenario: Scenario) async throws {
        let outcome = await ScenarioRunner(root: scenarioFolders).run(scenario)
        for failure in outcome.failures { Issue.record("\(scenario.id): \(failure)") }
        #expect(outcome.steps == scenario.steps.count, "\(scenario.id) ran every one of its steps")
        // What the plug-in alone cannot say of itself is the oracle's to compare, and said so in the log.
        print(
            "\(scenario.id): \(outcome.steps) steps, \(outcome.checks) checks made here, "
                + "\(outcome.withheld) comparisons with the oracle left to the desktop runner"
                + (outcome.rendersOn.isEmpty ? "" : ", rendered on \(JSONValue.array(outcome.rendersOn).shown)")
        )
    }
}
