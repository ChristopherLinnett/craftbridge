package io.github.christopherlinnett.craftbridge.testcount

/**
 * What a written test and a reported attempt agree on: the method names match, and the report's class is
 * the declaring class itself or a class under it. TestNG records a run under the concrete class it
 * instantiates, so a run the report attributes to a subclass answers for the class the test stands on.
 * Which concrete classes TestNG instantiates is Gradle's choice: it hands TestNG a class whose own
 * compiled class carries one of TestNG's annotations, or that a class-level `@Test` above it covers, and
 * no other — so a method-level `@Test` on an abstract base whose subclasses carry none of their own runs
 * nowhere, which is one of the never-run shapes the comparison exists to name.
 */
private fun matches(
    attempt: TestngAttempt,
    written: WrittenTest,
    isSubtypeOf: (String, String) -> Boolean,
): Boolean =
    attempt.methodName == written.methodName &&
        (attempt.className == written.className || isSubtypeOf(attempt.className, written.className))

/** The whole comparison: counts to print, and each line of blame for a count that does not close. */
class TestCountComparison(
    val written: Int,
    val ran: Int,
    val problems: List<String>,
) {
    val balanced: Boolean
        get() = problems.isEmpty()
}

/**
 * Compare the written tests against what TestNG reported running: every written test must stand behind
 * some test TestNG ran, and every test TestNG ran must stand on a written one. A written test TestNG
 * skipped comes back named with TestNG's own reason; a written test no attempt reaches at all is named
 * bare, which is how a `@Test` in an abstract class no run reaches, or one TestNG silently never runs,
 * is caught; and a run that left no report ([reported] false) while the build compiled tests says so
 * first. [isSubtypeOf] resolves the type hierarchy the test classes live in, so inherited tests match
 * their declaring class without a list of who inherits whom.
 */
fun compareWrittenAgainstRun(
    written: List<WrittenTest>,
    attempts: List<TestngAttempt>,
    unreadable: List<String>,
    reported: Boolean,
    isSubtypeOf: (String, String) -> Boolean,
): TestCountComparison {
    val distinct = attempts.distinctBy { it.className to it.methodName }
    val ranWritten = written.filter { w -> distinct.any { it.ran && matches(it, w, isSubtypeOf) } }
    val problems =
        buildList {
            if (!reported && written.isNotEmpty()) {
                add("  the test run left no $TESTNG_REPORT, so TestNG reported running none of them")
            }
            for (test in written.filterNot { it in ranWritten }) {
                val skip = distinct.firstOrNull { !it.ran && matches(it, test, isSubtypeOf) }
                add(
                    if (skip?.reason != null) {
                        "  TestNG never ran ${test.display}: it skipped it: ${skip.reason}"
                    } else {
                        "  TestNG never ran ${test.display}, and its report names no attempt at it"
                    },
                )
            }
            for (attempt in distinct.filterNot { a -> written.any { matches(a, it, isSubtypeOf) } }) {
                add("  TestNG ran ${attempt.display}, which no compiled class of the build marks as a test")
            }
            for (unread in unreadable) {
                add("  a compiled class of the build is unreadable: $unread")
            }
        }
    return TestCountComparison(written = written.size, ran = ranWritten.size, problems = problems)
}
