package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.tasks.TaskAction
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

/**
 * The Kotlin half of what `cargo xtask guide-samples` asks the builds: which Kotlin this module
 * compiles, and which lines of that Kotlin compile to nothing. The file list is AGP's own — every
 * folder of every component the variant API lists, the same folders every detekt run reads — so the
 * answer keeps no second list of folders (§3's rule that Kotlin sits only where a compilation reads
 * it, read from that one compilation list; §10). The dead lines are `CompilesToNothing`'s, from a
 * light detekt run over those files alone: every character of a reported line sits in a comment, a
 * KDoc, or a multi-line string, so no build compiles it, and a guide block quoted wholly from such
 * lines quotes prose dressed as code (AGENTS.md §10).
 *
 * The task writes its answer, one line each, into the folder xtask names in [GUIDE_ANSWERS]: `S`
 * and a file's path for a file the build compiles, then `I`, a file's path and a line number for
 * each line that compiles to nothing. A file of its own, never the console Gradle prints its own
 * lines to, so xtask can refuse every line it cannot read rather than pass over it as noise. Here
 * detekt's findings are the answer, not failures: the run's report is what the task reads.
 */
abstract class CraftbridgeGuideKotlin : CraftbridgeDetektRun() {
    @TaskAction
    fun answer() {
        // Read when the task runs, not when the build is configured, so each ask's own folder answers
        // without making the stored configuration stale.
        val folder =
            System.getenv(GUIDE_ANSWERS)?.let(::File)
                ?: error("$path: no $GUIDE_ANSWERS names the folder to answer into: cargo xtask guide-samples sets it when it asks")
        val compiled =
            checkedSources
                .files
                .flatMap { source ->
                    if (source.isDirectory) {
                        source.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
                    } else {
                        listOf(source)
                    }
                }.distinctBy { it.path }
                .sortedBy { it.path }
        val report = reportFile.get().asFile
        // An earlier run's report would answer for this one: only this run's own may.
        report.delete()
        val run =
            execOperations.javaexec {
                classpath = toolClasspath
                mainClass.set("dev.detekt.cli.Main")
                // A finding here is the answer, not a failure, so the exit code is read below rather than
                // by Gradle.
                isIgnoreExitValue = true
                args(
                    "--input",
                    checkedSources.joined(),
                    "--config",
                    configFiles.joined(),
                    "--plugins",
                    rulePlugins.joined(),
                    // Whether a line's characters sit in a comment or a string the parser alone answers:
                    // no rule of this run reads types, so no classpath joins the run.
                    "--analysis-mode",
                    "light",
                    // detekt's checkstyle report: the line-shaped one, `file` and `error line` elements
                    // naming exactly what the run found, read back below into `I` lines.
                    "--report",
                    "checkstyle:" + report.absolutePath,
                )
            }
        // detekt exits 0 when it found nothing and 2 when it found something, which here is the answer;
        // 1 is its own failure and 3 a config it refused, and neither run read the files.
        if (run.exitValue != 0 && run.exitValue != 2) {
            error("$path: detekt failed with exit code ${run.exitValue}, so no line of the files it was to read is answered")
        }
        if (!report.isFile) {
            error("$path: the detekt run wrote no report at ${report.path}: the run did not complete, and its findings are the answer")
        }
        val known = compiled.associateBy { it.invariantSeparatorsPath }
        val dead =
            deadLines(report)
                .mapKeys { (named, _) ->
                    resolve(known, named) ?: error("$path: the detekt report names $named, no file this module compiles")
                }
        val answer = StringBuilder()
        for (file in compiled) {
            answer.append("S\t${file.invariantSeparatorsPath}\n")
            for (line in dead[file.invariantSeparatorsPath].orEmpty()) {
                answer.append("I\t${file.invariantSeparatorsPath}\t$line\n")
            }
        }
        folder.mkdirs()
        // One file per task, named by its path, so the modules of one build never write into each other's.
        folder.resolve(path.removePrefix(":").replace(':', '.') + ".tsv").writeText(answer.toString())
    }

    /**
     * detekt may name a reported file from the run's own folder rather than absolutely: a name that is
     * not one of the compiled paths is taken as the compiled path it ends with, and only if exactly one
     * does — this run covered one module, so a relative name is its file's own path.
     */
    private fun resolve(
        known: Map<String, *>,
        named: String,
    ): String? {
        val name = named.removePrefix("./")
        known[name]?.let { return name }
        if (File(name).isAbsolute) {
            return null
        }
        return known.keys.filter { it.endsWith("/$name") }.singleOrNull()
    }

    /** The dead lines detekt reported, per file as the report named it, each file's lines in order. */
    private fun deadLines(report: File): Map<String, List<Int>> {
        val factory =
            DocumentBuilderFactory
                .newInstance()
                .apply {
                    // The report is detekt's own writing, and a report's external entity is still no
                    // file of Gradle's to fetch.
                    setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
                    setFeature("http://xml.org/sax/features/external-general-entities", false)
                    setFeature("http://xml.org/sax/features/external-parameter-entities", false)
                }
        val document =
            try {
                factory.newDocumentBuilder().parse(report)
            } catch (failure: Exception) {
                error("$path: the detekt report ${report.path} could not be read: ${failure.message}")
            }
        val files = document.getElementsByTagName("file")
        return buildMap {
            for (index in 0 until files.length) {
                val file = files.item(index) as? Element ?: continue
                val lines =
                    file
                        .getElementsByTagName("error")
                        .let { errors ->
                            (0 until errors.length)
                                .mapNotNull { errors.item(it) as? Element }
                                .filter { it.getAttribute("source").endsWith(".$GUIDE_RULE") }
                                .mapNotNull { it.getAttribute("line").toIntOrNull() }
                                .sorted()
                        }
                if (lines.isNotEmpty()) {
                    put(file.getAttribute("name"), lines)
                }
            }
        }
    }

    internal companion object {
        /** The one rule whose findings answer the question; the report may spell it with its rule set. */
        private const val GUIDE_RULE = "CompilesToNothing"

        /** The variable `cargo xtask guide-samples` names its answer folder in (`rust/xtask/src/samples.rs`). */
        const val GUIDE_ANSWERS = "CRAFTBRIDGE_GUIDE_ANSWERS"
    }
}
