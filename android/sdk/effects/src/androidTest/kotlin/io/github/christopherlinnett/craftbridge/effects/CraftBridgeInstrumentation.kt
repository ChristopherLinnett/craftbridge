// The checks that need a real device, and the runner `am instrument` starts them on.

package io.github.christopherlinnett.craftbridge.effects

import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.os.Bundle
import android.util.Log
import kotlinx.coroutines.runBlocking

/** The tag every line of a device run is written under, for whoever is reading with `logcat`. */
private const val LOG_TAG = "CraftBridge"

/** One check on a device, with the name it is reported under. */
internal class Case(
    internal val name: String,
    internal val check: suspend (Context) -> Unit,
)

/** Every check, in the order it runs: the folders first, since every engine starts in them. */
private val cases: List<Case> = folderChecks + engineChecks + wordsChecks

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

/** The key the run's end puts its failures under, for whoever reads the run's last report. */
private const val ERROR_KEY = "Error"

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
    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        // The platform calls only onCreate; onStart runs on the thread this starts. That thread is
        // where a check blocks on an engine, so the app's own main thread stays free while one runs.
        start()
    }

    override fun onStart() {
        super.onStart()
        val failures = mutableListOf<String>()
        for ((index, case) in cases.withIndex()) {
            Log.i(LOG_TAG, "checking: ${case.name}")
            // An empty line, since `am instrument` prints every key of a report that has no line at all.
            sendStatus(STARTED, report(case, index, ""))
            val refused = runCatching { runBlocking { case.check(targetContext) } }.exceptionOrNull()
            if (refused == null) {
                sendStatus(PASSED, report(case, index, "PASS  ${case.name}\n"))
            } else {
                failures += "${case.name}: ${refused.message}"
                Log.e(LOG_TAG, "failed:  ${case.name}", refused)
                sendStatus(
                    FAILED,
                    report(case, index, "FAIL  ${case.name}: ${refused.message}\n").apply {
                        putString(STACK_KEY, refused.stackTraceToString())
                    },
                )
            }
        }
        // The shape a JUnit runner ends a run with, written here because these checks have no JUnit
        // runner to write it: the line a person, or a script that does not read exit codes, looks for.
        val ended =
            if (failures.isEmpty()) {
                "OK (${cases.size} checks)"
            } else {
                "FAILURES!!! (${failures.size} of ${cases.size} checks refused)"
            }
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
        line: String,
    ): Bundle =
        Bundle().apply {
            // Not `javaClass`: the bundle is being built, not the runner.
            putString(CLASS_KEY, CraftBridgeInstrumentation::class.java.name)
            putString(TEST_KEY, case.name)
            putInt(CURRENT_KEY, index + 1)
            putInt(TOTAL_KEY, cases.size)
            putString(Instrumentation.REPORT_KEY_STREAMRESULT, line)
        }
}
