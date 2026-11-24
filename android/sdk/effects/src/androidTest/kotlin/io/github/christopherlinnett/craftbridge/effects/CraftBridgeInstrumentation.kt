// The checks that need a real device, and the runner `am instrument` starts them on.

package io.github.christopherlinnett.craftbridge.effects

import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.os.Bundle
import android.util.Log
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import kotlinx.coroutines.runBlocking
import java.util.regex.PatternSyntaxException

/** The tag every line of a device run is written under, for whoever is reading with `logcat`. */
internal const val LOG_TAG = "CraftBridge"

/** What the check in hand said for the run's reader, beside whether it passed: each line goes to logcat and into the check's report. */
private val said = mutableListOf<String>()

/** Says [line] in the report of the check that is running, and in logcat under [LOG_TAG]. */
internal fun say(line: String) {
    Log.i(LOG_TAG, line)
    said += line
}

/** The lines the check that has run said, one to a line, and nothing after: they are said once. */
private fun spoken(): String = said.joinToString("") { "$it\n" }.also { said.clear() }

/** One check on a device, with the name it is reported under, and the id of the corpus scenario it runs, if it runs one. */
internal class Case(
    internal val name: String,
    internal val scenario: String? = null,
    internal val check: suspend (Context) -> Unit,
)

/**
 * Every check, in the order it runs: the folders first, since every engine starts in them. The corpus's
 * checks are named by the scenarios the corpus holds, which are read from the test APK's assets, so they are
 * made once the app is there to read them. The checks are listed here by hand, so the count of the corpus's
 * scenarios is judged on this list, the one the run makes, and comes last.
 */
private fun cases(context: Context): List<Case> {
    val checks =
        folderChecks + engineChecks + wordsChecks + hardwareDecodingChecks + fp16FloorChecks + exportPoolChecks + frameChecks +
            previewChecks + apiChecks + typedCallChecks + boundarySmokeChecks + selectionChecks + corpusModelChecks +
            scenarioRunnerChecks + conformanceChecks(context) + plugInPolicyChecks + memoryGuardChecks + failedExportChecks + baselineChecks
    return checks + scenarioCountCheck(checks)
}

/**
 * The words of the instrumentation test protocol, which `am instrument -r` prints and every reader of a
 * test run (Gradle's and Android Studio's among them) reads: the keys that name a check, count the run
 * and carry a failure's trace, and the code of each report. They are the public constants of the
 * platform's `android.test.InstrumentationTestRunner`, written out because the optional library that
 * holds them is not on these checks' classpath. A report under any other code is read as some other
 * outcome: the protocol's -1, `Activity.RESULT_OK`, is an error.
 */
private const val CLASS_KEY = "class"
private const val TEST_KEY = "test"
private const val CURRENT_KEY = "current"
private const val TOTAL_KEY = "numtests"
private const val STACK_KEY = "stack"
private const val STARTED = 1
private const val PASSED = 0
private const val FAILED = -2

/**
 * The run argument that narrows a run to the checks whose name matches the regular expression it holds
 * (`am instrument -e only "T22|T24"`), for whoever is working on some checks and does not want to wait for the
 * rest. Without it every check runs.
 */
private const val ONLY_KEY = "only"

/**
 * The checks a run makes: [all] of them, or with [only] those whose name the regular expression matches. A
 * pattern that is no regular expression, or that matches no check's name, is one check that fails saying so,
 * since a run that made no check would end `OK`, which reads as every check passing.
 */
internal fun chosen(
    all: List<Case>,
    only: String?,
): List<Case> {
    if (only == null) {
        return all
    }
    val asked = "the checks `-e $ONLY_KEY $only` names"
    val pattern = runCatching { Regex(only) }.getOrElse { refused -> return listOf(Case(asked) { throw refused }) }
    return all.filter { case -> pattern.containsMatchIn(case.name) }.ifEmpty { listOf(Case(asked) { error("no check's name matches it") }) }
}

/**
 * The line a run of [made] checks ends with, [refused] of them failing: the shape a JUnit runner ends a run with,
 * written here because these checks have no JUnit runner to write it, the line a person, or a script that does not
 * read exit codes, looks for. After a failure on a device that gives the engine no GPU adapter it says why
 * ([noGpuAdapter]), so the failures of the checks that render on the GPU read as the device's gap.
 */
internal fun runEnd(
    refused: Int,
    made: Int,
    noGpuAdapter: String?,
): String {
    if (refused == 0) {
        return "OK ($made checks)"
    }
    val gap =
        noGpuAdapter?.let { why ->
            "; this device gives the engine no GPU adapter, which every check that renders on the GPU needs: $why"
        }
    return "FAILURES!!! ($refused of $made checks refused${gap.orEmpty()})"
}

/** The checks of how a run is narrowed, and of how it ends. */
internal val selectionChecks: List<Case> =
    listOf(
        Case("a run ends OK with its count, or with its failures counted and why the device gives the engine no GPU adapter") {
            agreed(runEnd(0, 74, "no adapter: none found"), "OK (74 checks)", "a run with no failure")
            agreed(runEnd(1, 74, null), "FAILURES!!! (1 of 74 checks refused)", "a failure on a device with a GPU adapter")
            agreed(
                runEnd(9, 74, "no adapter: none found"),
                "FAILURES!!! (9 of 74 checks refused; this device gives the engine no GPU adapter, " +
                    "which every check that renders on the GPU needs: no adapter: none found)",
                "failures on a device with no GPU adapter",
            )
        },
        Case(
            "`-e only` narrows a run to the checks it matches, and a pattern that matches none or is no pattern fails the run",
        ) { context ->
            val all = listOf("alpha one", "beta", "alpha two").map { name -> Case(name) {} }
            agreed(chosen(all, null).map { it.name }, all.map { it.name }, "a run with no pattern")
            agreed(chosen(all, "^alpha").map { it.name }, listOf("alpha one", "alpha two"), "the checks a pattern matches")
            val none = chosen(all, "no such check").single()
            agreed(none.name, "the checks `-e only no such check` names", "the one check of a pattern that matches none")
            val unmatched = runCatching { none.check(context) }.exceptionOrNull()
            agreed(unmatched?.message, "no check's name matches it", "why a pattern that matches none fails")
            val broken = chosen(all, "[").single()
            agreed(broken.name, "the checks `-e only [` names", "the one check of a pattern that is no regular expression")
            val refused = runCatching { broken.check(context) }.exceptionOrNull()
            require(refused is PatternSyntaxException) { "a pattern that is no regular expression failed with $refused" }
        },
    )

/** The key the run's end puts its failures under, for whoever reads the run's last report. */
private const val ERROR_KEY = "Error"

/**
 * Why this device gives the engine no GPU adapter, in EffectCraft's words, or `null` where it gives one. Every check
 * that renders on the GPU fails on such a device, which is the device's gap, not the plug-in's, so a run's end says
 * it beside the failures; an adapter search that broke on a phone with a GPU says it there too, and still fails.
 */
private suspend fun noGpuAdapter(context: Context): String? {
    val engine = EffectsEngine.start(context, gpu = GpuChoice.PREFERRED)
    try {
        return ScenarioRunner.noAdapter(engine.request("info", JSONValue.Object(emptyMap())))
    } finally {
        engine.shutDown()
    }
}

/**
 * The device checks' runner. The usual runner for instrumented checks is androidx.test's, which needs
 * JUnit 4, and JUnit 4's EPL-1.0 is on no branch of the licence allowlist (rust/deny.toml) that the
 * licence check reads the test classpaths against — so these checks are run by the platform's own
 * [Instrumentation], which reports each check in the instrumentation test protocol as androidx.test does:
 * its start, then its pass or its failure with the failure's trace.
 *
 * Every check is a coroutine: a check that ends a call over the boundary waits for the engine, and a
 * check of a cancel cancels a real coroutine.
 */
public class CraftBridgeInstrumentation : Instrumentation() {
    private var only: String? = null

    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        only = arguments?.getString(ONLY_KEY)
        // The platform calls only onCreate; onStart runs on the thread this starts. That thread is
        // where a check blocks on an engine, so the app's own main thread stays free while one runs.
        start()
    }

    override fun onStart() {
        super.onStart()
        val failures = mutableListOf<String>()
        val cases = chosen(cases(targetContext), only)
        for ((index, case) in cases.withIndex()) {
            Log.i(LOG_TAG, "checking: ${case.name}")
            // An empty line, since `am instrument` prints every key of a report that has no line at all.
            sendStatus(STARTED, report(case, index, cases.size, ""))
            val refused = runCatching { runBlocking { case.check(targetContext) } }.exceptionOrNull()
            val heard = spoken()
            if (refused == null) {
                sendStatus(PASSED, report(case, index, cases.size, "PASS  ${case.name}\n$heard"))
            } else {
                failures += "${case.name}: ${refused.message}"
                Log.e(LOG_TAG, "failed:  ${case.name}", refused)
                sendStatus(
                    FAILED,
                    report(case, index, cases.size, "FAIL  ${case.name}: ${refused.message}\n$heard").apply {
                        putString(STACK_KEY, refused.stackTraceToString())
                    },
                )
            }
        }
        // Asked only after a failure; an engine that does not start says nothing of the GPU, and the checks' own
        // failures say why.
        val gpuGap = if (failures.isEmpty()) null else runCatching { runBlocking { noGpuAdapter(targetContext) } }.getOrNull()
        val ended = runEnd(failures.size, cases.size, gpuGap)
        Log.i(LOG_TAG, ended)
        // The line goes with the run's end, not as a report of its own: a report names a check, and one
        // that names none is read as no check's, or not at all.
        val results =
            Bundle().apply {
                putString(Instrumentation.REPORT_KEY_STREAMRESULT, "$ended\n")
                if (failures.isNotEmpty()) {
                    putString(ERROR_KEY, failures.joinToString("\n"))
                }
            }
        finish(if (failures.isEmpty()) Activity.RESULT_OK else Activity.RESULT_FIRST_USER, results)
    }

    /**
     * One report about the check at `index`: what names it to a reader of the protocol, and `line`, the
     * text `am instrument` prints for it as it goes, which a report needs a line break at the end of.
     */
    private fun report(
        case: Case,
        index: Int,
        total: Int,
        line: String,
    ): Bundle =
        Bundle().apply {
            // Not `javaClass`: the bundle is being built, not the runner.
            putString(CLASS_KEY, CraftBridgeInstrumentation::class.java.name)
            putString(TEST_KEY, case.name)
            putInt(CURRENT_KEY, index + 1)
            putInt(TOTAL_KEY, total)
            putString(Instrumentation.REPORT_KEY_STREAMRESULT, line)
        }
}
