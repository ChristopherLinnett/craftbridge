// Runs the plug-in's side of a corpus scenario on a phone: each step's calls made on an engine the SDK
// starts over the scenario's own folder, and what the corpus says of the result checked. The oracle, the
// EffectCraft command line, is a program of the desktop, so what the corpus holds the two sides to each
// other for (`same`, and the comparisons of files) is the desktop runner's; here a comparison's file must
// be there and hold something. The one comparison a phone can make on its own is the compositor's GPU
// tolerance: the oracle renders on the CPU, so a scenario that asks for the GPU is made a second time on
// the CPU and its files are held to that. Every other check, the expectations and what the folders hold,
// is the corpus's own and is made as the desktop makes it.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.InvalidParamsException
import io.github.christopherlinnett.craftbridge.core.IoException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import java.io.File

/** Where the scenario's folder goes in a text of the corpus: the engine takes absolute paths, and the folder is not known until the scenario runs. */
private const val CONTAINER = "<container>"

/** What one scenario came to: the failures it found, every one, and how much it ran and checked. */
internal class Outcome {
    val failures = mutableListOf<String>()
    var steps = 0

    /** The expectations and folder rules made, the files a comparison names that were there, and the files held to the CPU's render. */
    var checks = 0

    /** The comparisons with the oracle's output or files, which the desktop runner makes. */
    var withheld = 0

    /** Where each step that asked for the GPU rendered, as the engine's `info` says, and then where each rendered when the scenario was made on the CPU. */
    val rendersOn = mutableListOf<JSONValue>()
}

/** What one step printed: its documents, the failure that ended it if one did, and where it rendered if it asked for the GPU. */
private class Printed(
    val documents: List<JSONValue>,
    val failed: CraftException?,
    val rendersOn: JSONValue?,
)

/** This value with [text] written wherever a string holds [target]. */
private fun JSONValue.replacing(
    target: String,
    text: String,
): JSONValue =
    when (this) {
        is JSONValue.Text -> JSONValue.Text(value.replace(target, text))
        is JSONValue.Array -> JSONValue.Array(values.map { it.replacing(target, text) })
        is JSONValue.Object -> JSONValue.Object(entries.mapValues { (_, entry) -> entry.replacing(target, text) })
        else -> this
    }

/** Names as a failure lists them, each quoted, as the Swift runner's failures list them. */
internal fun listed(names: List<String>): String = names.joinToString(", ", "[", "]") { "\"$it\"" }

/** How many items an array or an object holds, or null for any other value. */
private val JSONValue.itemCount: Int?
    get() =
        when (this) {
            is JSONValue.Array -> values.size
            is JSONValue.Object -> entries.size
            else -> null
        }

/**
 * The folders of an app whose container is [container]: the working folders sit inside it, each hidden from
 * a listing of it by a leading dot, and the plug-in folder the app names, relative to the container, is
 * [plugins]. The engine refuses folders outside the container, so a scenario's folder lies in the app's own data.
 */
internal fun engineFolders(
    container: File,
    plugins: String?,
): Folders =
    Folders(
        container = container.path,
        bundle = null,
        config = File(container, ".config").path,
        cache = File(container, ".cache").path,
        models = File(container, ".models").path,
        staged = File(container, ".staged").path,
        plugins = plugins?.let { File(container, it).path },
        fonts = emptyList(),
    )

/**
 * The plug-in side of the corpus, run over scenario folders made in [root], which lies in the app's own
 * data folder and which the caller removes when it is done with it.
 */
internal class ScenarioRunner(
    private val context: Context,
    private val root: File,
) {
    /**
     * Runs every step of [scenario] in a folder of its own in [root], removed when it ends, and checks what the
     * corpus says of the outputs and the folder. Every failure is found, not only the first: a step that cannot
     * run ends the scenario, since what it should have written is not there to check. A scenario held to the
     * CPU's render ([heldToTheCPU]) is made a second time, on the CPU, in a folder of its own.
     */
    suspend fun run(scenario: Scenario): Outcome {
        val outcome = Outcome()
        val folder = File(root, scenario.id)
        val onTheCPU = if (heldToTheCPU(scenario)) File(root, "${scenario.id} on the CPU") else null
        try {
            val (outputs, failed, rendersOn) = steps(scenario, folder, gpu = true)
            outcome.failures += failed
            outcome.rendersOn += rendersOn
            if (onTheCPU != null) {
                val (_, reference, referenceRendersOn) = steps(scenario, onTheCPU, gpu = false)
                outcome.failures += reference.map { "on the CPU: $it" }
                outcome.rendersOn += referenceRendersOn
            }
            outcome.steps = outputs.size
            outcome.failures += scenario.expects.mapNotNull { failure(it, outputs) }
            outcome.failures += scenario.folders.mapNotNull { failure(it, folder) }
            outcome.failures += scenario.compares.flatMap { failures(it, folder, onTheCPU) }
            val madeHere = if (onTheCPU == null) 0 else scenario.compares.count { it.pixels != null }
            outcome.checks = scenario.expects.size + scenario.folders.size + scenario.compares.size + madeHere
            outcome.withheld = scenario.sames.size + scenario.compares.size - madeHere
        } catch (unjudged: RunnerFailure) {
            outcome.failures += unjudged.description
        } finally {
            for (made in listOfNotNull(folder, onTheCPU)) {
                made.deleteRecursively()
            }
        }
        return outcome
    }

    /**
     * Every step of [scenario] made in [folder], which starts with the scenario's fixtures, each step on the GPU
     * its host asks for unless [gpu] is false: what each step printed, the failures of the plug-in, and where
     * each step that asked for the GPU rendered. Without the oracle to say a call should fail, only the
     * scenario's own reading of its failure (`/kind`) makes one an answer; any other is the plug-in failing.
     */
    private suspend fun steps(
        scenario: Scenario,
        folder: File,
        gpu: Boolean,
    ): Triple<List<List<JSONValue>>, List<String>, List<JSONValue>> {
        folder.mkdirs()
        for (fixture in scenario.fixtures) {
            place(fixture, folder)
        }
        val outputs = mutableListOf<List<JSONValue>>()
        val failures = mutableListOf<String>()
        val rendersOn = mutableListOf<JSONValue>()
        for ((index, step) in scenario.steps.withIndex()) {
            val printed =
                try {
                    run(step, folder, gpu && step.host?.gpu == true)
                } catch (unjudged: RunnerFailure) {
                    throw RunnerFailure("step $index: ${unjudged.description}")
                }
            outputs.add(printed.documents)
            printed.rendersOn?.let { rendersOn += it }
            val read = scenario.expects.any { it.step == index && it.pointer == "/kind" }
            if (printed.failed != null && !read) {
                failures += "step $index: the plug-in failed: ${printed.failed.message.orEmpty()}"
            }
        }
        return Triple(outputs, failures, rendersOn)
    }

    /** A fixture copied from the corpus's assets into the scenario's folder. */
    internal fun place(
        fixture: Fixture,
        folder: File,
    ) {
        val to = File(folder, fixture.to)
        to.parentFile?.mkdirs()
        val bytes = context.assets.open(fixture.from).use { it.readBytes() }
        if (fixture.namesContainer) {
            to.writeText(bytes.decodeToString().replace(CONTAINER, folder.path))
        } else {
            to.writeBytes(bytes)
        }
    }

    /**
     * One step: an engine started afresh over the scenario's folder, on the GPU when [gpu] says, the step's
     * calls made on it in order, and the documents of the last call's answer, or of the first failure, which
     * ends the step, with where the engine renders when the step's host asks for the GPU.
     */
    private suspend fun run(
        step: Step,
        folder: File,
        gpu: Boolean,
    ): Printed {
        val engine =
            try {
                EffectsEngine.start(
                    context,
                    engineFolders(folder, step.host?.plugins),
                    if (gpu) GpuChoice.PREFERRED else GpuChoice.CPU_ONLY,
                )
            } catch (refused: CraftException) {
                return Printed(listOf(documentOf(refused)), refused, null)
            }
        try {
            var rendersOn: JSONValue? = null
            if (gpu || step.host?.gpu == true) {
                val info = engine.request("info", JSONValue.Object(emptyMap()))
                failure(info, gpu)?.let { throw it }
                rendersOn = info.valueAt("/gpu") ?: JSONValue.Null
            }
            var printed: List<JSONValue> = emptyList()
            for (call in step.calls) {
                try {
                    printed = make(call, engine, folder, step.stdin)
                } catch (refused: CraftException) {
                    return Printed(listOf(documentOf(refused)), refused, rendersOn)
                }
            }
            return Printed(printed, null, rendersOn)
        } finally {
            // An engine that cannot end has nothing a scenario could say of it; its thread ends with the process.
            runCatching { engine.shutDown() }
        }
    }

    /** One call's answer as the documents its step prints. */
    private suspend fun make(
        call: Call,
        engine: EffectsEngine,
        folder: File,
        input: List<String>,
    ): List<JSONValue> {
        val at = folder.path
        return when (call) {
            is Call.Request -> {
                var sent = call.params.replacing(CONTAINER, at)
                for ((name, path) in call.files) {
                    val fields =
                        (sent as? JSONValue.Object)?.entries
                            ?: throw InvalidParamsException(
                                "`${call.method}`: parameters that take a file must be an object",
                                "`${call.method}`: parameters that take a file must be an object",
                            )
                    val text =
                        try {
                            File(folder, path).readText()
                        } catch (unreadable: java.io.IOException) {
                            throw IoException("cannot read $path: ${unreadable.message}", "cannot read $path: ${unreadable.message}")
                        }
                    sent = JSONValue.Object(fields + (name to JSONValue.Text(text)))
                }
                val stop = call.progress
                listOf(if (stop == null) engine.request(call.method, sent) else Listening(stop).run(call.method, sent, engine))
            }

            is Call.Frame -> {
                val straight = call.format == FrameFormat.RGBA8
                val frame =
                    engine.core.frame(
                        jsonText(call.params.replacing(CONTAINER, at)),
                        if (straight) PixelFormat.RGBA8_STRAIGHT else PixelFormat.PNG,
                    )
                val bytes = if (straight) pngOfStraight(frame.data, frame.width.toInt(), frame.height.toInt()) else frame.data
                try {
                    File(folder, call.out).writeBytes(bytes)
                } catch (unwritable: java.io.IOException) {
                    throw IoException("cannot write ${call.out}: ${unwritable.message}", "cannot write ${call.out}: ${unwritable.message}")
                }
                listOf(
                    JSONValue.Object(
                        mapOf("width" to JSONValue.Number(frame.width.toDouble()), "height" to JSONValue.Number(frame.height.toDouble())),
                    ),
                )
            }

            is Call.Mcp -> {
                input.mapNotNull { line -> engine.mcp(line.replace(CONTAINER, at))?.let(::jsonValueOf) }
            }
        }
    }

    internal companion object {
        /**
         * Whether a phone holds the scenario's files to the CPU's render: its steps ask for the GPU, and a
         * comparison holds a file to the compositor's tolerance for a render on the GPU against the CPU's,
         * which is how the oracle renders.
         */
        fun heldToTheCPU(scenario: Scenario): Boolean =
            scenario.steps.any { it.host?.gpu == true } && scenario.compares.any { it.pixels != null }

        /**
         * Why a step cannot be judged, given the engine's `info`, when it does not render where [onTheGPU] says:
         * a step that asked for the GPU on a machine with none would run on the CPU and pass, and a CPU's render
         * made on the GPU would hold the GPU's render to itself. The engine renders on the GPU when its session
         * names the adapter it holds (`/project/renderer/adapter`, EffectCraft's own record). `/gpu` names the
         * adapter or says in words why there is none, a start that found no adapter included, so it alone cannot
         * tell the two apart.
         */
        fun failure(
            info: JSONValue,
            onTheGPU: Boolean,
        ): RunnerFailure? {
            val adapter = info.valueAt("/project/renderer/adapter") ?: JSONValue.Null
            return when {
                onTheGPU && adapter is JSONValue.Text -> {
                    null
                }

                !onTheGPU && adapter is JSONValue.Null -> {
                    null
                }

                onTheGPU -> {
                    RunnerFailure(
                        "the step needs the GPU, and the engine renders on the CPU: " +
                            (info.valueAt("/gpu")?.shown ?: "it says nothing of why"),
                    )
                }

                else -> {
                    RunnerFailure("the step's render on the CPU is on the GPU, ${adapter.shown}")
                }
            }
        }

        /** The failure of one expectation, if it does not hold. */
        fun failure(
            expect: Expect,
            outputs: List<List<JSONValue>>,
        ): String? {
            val at = "step ${expect.step} ${expect.pointer}"
            val documents = outputs.getOrNull(expect.step) ?: return "$at: the step did not run"
            val document =
                documents.getOrNull(expect.output) ?: return "$at: the step printed ${documents.size} documents, none at ${expect.output}"
            val found = document.valueAt(expect.pointer) ?: return "$at: the output has no such value"
            if (expect.length != null) {
                val held = found.itemCount ?: return "$at: found ${found.shown}, which holds no items"
                return if (held == expect.length) null else "$at: holds $held items, expected ${expect.length}"
            }
            return if (found == expect.equals) null else "$at: found ${found.shown}, expected ${expect.equals?.shown ?: "none"}"
        }

        /**
         * The failure of one folder rule, if the folder does not hold exactly the names it says: the entries not
         * starting with a dot, which are the engine's own working files.
         */
        fun failure(
            rule: Folder,
            folder: File,
        ): String? {
            val held = File(folder, rule.path).list() ?: return "${rule.path}: cannot read the folder"
            val names = held.filter { !it.startsWith(".") }.sorted()
            return if (names ==
                rule.holds.sorted()
            ) {
                null
            } else {
                "${rule.path} holds ${listed(names)}, expected ${listed(rule.holds.sorted())}"
            }
        }

        /**
         * The failures of one comparison of files, as far as a phone makes it: the plug-in's file is there and
         * holds something, and, when [onTheCPU] holds the scenario made on the CPU, a file held to the GPU's
         * tolerance is within it of the CPU's. What else it is held against is the oracle's, which only the
         * desktop has.
         */
        fun failures(
            compare: Compare,
            folder: File,
            onTheCPU: File? = null,
        ): List<String> {
            val file = File(folder, compare.actual)
            if (!file.exists()) {
                return listOf("${compare.actual}: the step wrote no such file")
            }
            val held = if (file.isDirectory) file.list()?.size else file.length().toInt()
            if ((held ?: 0) <= 0) {
                return listOf("${compare.actual}: is empty")
            }
            val tolerance = compare.pixels
            if (tolerance == null || onTheCPU == null) {
                return emptyList()
            }
            return pixelFailures(file, File(onTheCPU, compare.actual), tolerance).map { "${compare.actual} against the CPU's: $it" }
        }
    }
}
