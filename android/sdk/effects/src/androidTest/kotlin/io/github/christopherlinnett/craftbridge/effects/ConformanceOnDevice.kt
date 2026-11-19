// The conformance corpus on the phone (plan §28, QA-2): every scenario of `conformance/effects/corpus.toml`
// the plug-in runs, made through the SDK on the engine it starts, one check each, named by the scenario's id
// and title as the Swift run names its cases. The corpus is the one file the desktop runner reads, packed into
// the test APK from where it is, and never copied.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.util.Log
import io.github.christopherlinnett.craftbridge.core.JSONValue
import java.io.File
import java.io.IOException

/** The corpus file in the test APK's assets, which are the corpus's own folder. */
private const val CORPUS_ASSET = "corpus.toml"

/** The folder of the app's own data the scenarios' folders are made in, which an engine's folders must lie inside. */
private const val SCENARIO_FOLDER = "craftbridge-conformance"

/** The folder the scenarios' folders are made in, which lies in the app's own data folder. */
internal fun scenarioRoot(context: Context): File = File(context.dataDir, SCENARIO_FOLDER)

/** The text of the file [name] in the test APK's assets, which are the conformance folder's own files. */
internal fun assetText(
    context: Context,
    name: String,
): String = context.assets.open(name).use { it.readBytes().decodeToString() }

/** The corpus file's text. */
internal fun corpusText(context: Context): String = assetText(context, CORPUS_ASSET)

/** The scenarios the plug-in runs, which is every one the oracle does not run alone. */
internal fun pluginScenarios(text: String): List<Scenario> = corpusOf(text).scenarios.filter { it.sides.runsPlugin }

/** What a scenario's check logs once it has run, for whoever reads the run with `logcat`. */
private fun summaryOf(
    scenario: Scenario,
    outcome: Outcome,
): String =
    "${scenario.id}: ${outcome.steps} steps, ${outcome.checks} checks made here, " +
        "${outcome.withheld} comparisons with the oracle left to the desktop runner" +
        if (outcome.rendersOn.isEmpty()) "" else ", rendered on ${JSONValue.Array(outcome.rendersOn).shown}"

/** The check of one scenario: its steps made through the SDK, and every failure of what the corpus says of them. */
private fun scenarioCheck(scenario: Scenario): Case =
    Case(scenario.checkName, scenario.id) { context ->
        val root = scenarioRoot(context)
        try {
            val outcome = ScenarioRunner(context, root).run(scenario)
            Log.i(LOG_TAG, summaryOf(scenario, outcome))
            require(outcome.failures.isEmpty()) { outcome.failures.joinToString("\n") { "${scenario.id}: $it" } }
            require(outcome.steps == scenario.steps.size) { "${scenario.id} ran ${outcome.steps} of its ${scenario.steps.size} steps" }
        } finally {
            // Only when every scenario's own folder is gone: a folder that still holds something is left alone.
            root.delete()
        }
    }

/**
 * The check that the Kotlin run takes every scenario the plug-in runs, once each and in the corpus's order. The
 * scenarios are counted from the TOML document itself, not through the model or its filter, against the scenario
 * checks of [checks], the list the run makes, so a scenario the model, its filter or the list loses is named.
 */
internal fun scenarioCountCheck(checks: List<Case>): Case =
    Case("the corpus parses and the Kotlin run takes every scenario the plug-in runs") { context ->
        val corpus = pluginScenarioIds(corpusText(context))
        val run = checks.mapNotNull { it.scenario }
        require(corpus.isNotEmpty()) { "the corpus holds scenarios the plug-in runs" }
        val left = corpus.filter { it !in run }
        val added = run.filter { it !in corpus }
        require(run == corpus) { "the Kotlin run leaves out ${listed(left)} and adds ${listed(added)}, in the order ${listed(run)}" }
        Log.i(LOG_TAG, "the Kotlin run takes ${run.size} scenarios, of the corpus's ${corpus.size} that the plug-in runs")
    }

/** The check that a scenario's folder is staged in the app's own data folder, where an engine's folders must be. */
private val rootCheck =
    Case("a scenario's folder is staged in the app's own data folder") { context ->
        val data = context.dataDir.canonicalPath
        val root = scenarioRoot(context).canonicalPath
        require(root.startsWith("$data/")) { "the scenarios' folders are made in $root, outside the app's data folder $data" }
    }

/**
 * The corpus's checks: the folder they are staged in, then one for each scenario the plug-in runs, which
 * [scenarioCountCheck] counts. A corpus the runner cannot read is one check that fails with why, since there
 * are then no scenarios to name.
 */
internal fun conformanceChecks(context: Context): List<Case> =
    try {
        listOf(rootCheck) + pluginScenarios(corpusText(context)).map(::scenarioCheck)
    } catch (unread: RunnerFailure) {
        listOf(Case("the corpus parses") { throw unread })
    } catch (unread: IOException) {
        listOf(Case("the corpus is in the test APK") { error("the test APK holds no $CORPUS_ASSET in its assets: ${unread.message}") })
    }
