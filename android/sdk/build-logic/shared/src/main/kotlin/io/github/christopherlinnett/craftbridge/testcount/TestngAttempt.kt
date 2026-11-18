package io.github.christopherlinnett.craftbridge.testcount

import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

/** The file TestNG's own `XMLReporter` writes into its output folder, one for each TestNG run. */
internal const val TESTNG_REPORT = "testng-results.xml"

/**
 * One test TestNG reported, as its own report file names it: a class, a method, and the run's outcome —
 * PASS or FAIL mean the method ran, SKIP means TestNG never ran it and [reason] says why. A data-driven
 * test appears once per invocation; the comparison asks only whether a method was run at all, so those
 * collapse to one entry each, which [readTestngReports] does.
 */
data class TestngAttempt(
    val className: String,
    val methodName: String,
    val status: String,
    val reason: String?,
) {
    /** Whether TestNG ran the method: a skipped one says it never reached it, whatever else it says. */
    val ran: Boolean
        get() = status == "PASS" || status == "FAIL"

    /** The name a report line gives this attempt: the report's own class and method, as it recorded them. */
    val display: String
        get() = "$className.$methodName"
}

/**
 * Read TestNG's own [TESTNG_REPORT] — the file its `XMLReporter` writes, which is what every test task
 * of this build lets TestNG report through — into one attempt per distinct class and method.
 * Configuration methods are no tests and are left out, as the report marks them; a method TestNG
 * skipped carries the message of its exception as the reason, on one line, which is TestNG naming why
 * it never ran the method, not this check guessing. Files are read in name order so the report never
 * depends on a file system's mood.
 */
fun readTestngReports(files: List<File>): List<TestngAttempt> {
    val attempts = LinkedHashMap<Pair<String, String>, TestngAttempt>()
    for (file in files.sortedBy { it.name }) {
        val document = documentingFactory().newDocumentBuilder().parse(file)
        val classes = document.getElementsByTagName("class")
        for (c in 0 until classes.length) {
            val classNode = classes.item(c) as Element
            val methods = classNode.childNodes
            for (m in 0 until methods.length) {
                val method = methods.item(m) as? Element ?: continue
                if (method.tagName != "test-method" || method.getAttribute("is-config") == "true") {
                    continue
                }
                val attempt =
                    TestngAttempt(
                        className = classNode.getAttribute("name"),
                        methodName = method.getAttribute("name"),
                        status = method.getAttribute("status"),
                        reason = method.messageOrNull(),
                    )
                val key = attempt.className to attempt.methodName
                // Ran beats skipped beats skipped-with-no-reason, so a method that ran in one
                // invocation and skipped in another is known to have run.
                val kept = attempts[key]
                if (kept == null || attemptPrecedence(attempt) > attemptPrecedence(kept)) {
                    attempts[key] = attempt
                }
            }
        }
    }
    return attempts.values.sortedWith(compareBy({ it.className }, { it.methodName }))
}

/** A running test outranks a skipped one, and either outranks a skip whose reason the report left out. */
private fun attemptPrecedence(attempt: TestngAttempt): Int =
    when {
        attempt.ran -> 2
        attempt.reason != null -> 1
        else -> 0
    }

private fun documentingFactory(): DocumentBuilderFactory =
    DocumentBuilderFactory
        .newInstance()
        .apply {
            // The reports are TestNG's own files, but a gate reader takes no document's word for it:
            // no external entity reaches outside the report folder through a parsed report.
            setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
            isXIncludeAware = false
            isNamespaceAware = false
        }

/** The first `<message>` the report wrote under a method — TestNG's own words — on one line, if any. */
private fun Element.messageOrNull(): String? {
    val messages = getElementsByTagName("message")
    for (i in 0 until messages.length) {
        val text =
            messages
                .item(i)
                .textContent
                ?.trim()
                ?.replace(WHITESPACE, " ")
        if (!text.isNullOrEmpty()) {
            return text
        }
    }
    return null
}

private val WHITESPACE = Regex("\\s+")
