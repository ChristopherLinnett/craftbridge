// What the runner holds a scenario to, each rule pinned in its own words: a failure names where it was
// found, a folder holds exactly the names it is told, a comparison's file must hold something, a file held
// to the GPU's tolerance is held to the CPU's render, a plug-in that fails is no answer unless the scenario
// reads its failure, and a stop the export did not meet is reported as a race, never passed.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import android.graphics.Bitmap
import io.github.christopherlinnett.craftbridge.core.JSONValue
import java.io.File
import java.util.UUID

/** The scenario [calls] and [rest] make, in the corpus's own text: one step, whose calls are [calls]. */
private fun scenarioWith(
    calls: String,
    rest: String = "",
): Scenario = scenarioOf("[[scenario]]\nid = \"X\"\ntitle = \"x\"\n[[scenario.step]]\n$calls\n$rest")

private const val INFO = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n"

/** A new folder in the app's cache holding [files], each by its path in the folder, and removed once [use] has run. */
private suspend fun <T> holding(
    context: Context,
    files: Map<String, ByteArray>,
    use: suspend (File) -> T,
): T {
    val folder = File(context.cacheDir, "craftbridge-runner-${UUID.randomUUID()}")
    try {
        folder.mkdirs()
        for ((path, bytes) in files) {
            File(folder, path).apply { parentFile?.mkdirs() }.writeBytes(bytes)
        }
        return use(folder)
    } finally {
        folder.deleteRecursively()
    }
}

/** What the runner makes of [scenario], made in a folder of the app's cache that is removed afterwards. */
private suspend fun outcomeOf(
    context: Context,
    scenario: Scenario,
): Outcome = holding(context, emptyMap()) { root -> ScenarioRunner(context, root).run(scenario) }

private fun held(vararg names: Pair<String, String>): Map<String, ByteArray> =
    names.associate { (path, text) -> path to text.toByteArray() }

/** A PNG file of [width] by [height] pixels in [folder], each pixel the straight RGBA [shade] gives it. */
private fun pngFile(
    name: String,
    folder: File,
    width: Int,
    height: Int,
    shade: (Int) -> List<Int>,
): File =
    File(folder, name).apply {
        writeBytes(pngOfStraight(ByteArray(width * height * 4) { shade(it / 4)[it % 4].toByte() }, width, height))
    }

/** The scenario's own expectations, held to what a step printed: every one fails in its own words. */
private val expectationChecks =
    Case("an expectation that does not hold fails and says where and what was found") {
        val rules =
            scenarioWith(
                INFO,
                listOf(
                    "step = 0\npointer = \"/version\"\nequals = \"0.0.0\"",
                    "step = 0\npointer = \"/version\"\nlength = 3",
                    "step = 0\npointer = \"/items\"\nlength = 3",
                    "step = 0\npointer = \"/nothing\"\nequals = 1",
                    "step = 1\npointer = \"\"\nequals = 1",
                    "step = 0\noutput = 4\npointer = \"\"\nequals = 1",
                    "step = 0\npointer = \"/version\"\nequals = \"0.24.1\"",
                    "step = 0\npointer = \"/items\"\nlength = 2",
                    "step = 0\npointer = \"/n\"\nequals = 90",
                ).joinToString("") { "[[scenario.expect]]\n$it\n" },
            )
        val outputs =
            listOf(listOf(obj("version" to str("0.24.1"), "items" to JSONValue.Array(listOf(num(1.0), num(2.0))), "n" to num(90.0))))
        val found = rules.expects.mapNotNull { ScenarioRunner.failure(it, outputs) }
        agreed(
            found,
            listOf(
                "step 0 /version: found \"0.24.1\", expected \"0.0.0\"",
                "step 0 /version: found \"0.24.1\", which holds no items",
                "step 0 /items: holds 2 items, expected 3",
                "step 0 /nothing: the output has no such value",
                "step 1 : the step did not run",
                "step 0 : the step printed 1 documents, none at 4",
            ),
            "the failures",
        )
    }

private val folderRuleCheck =
    Case("a folder holds exactly the names it is told and the engine's dot files do not count") { context ->
        val rules =
            scenarioWith(
                INFO,
                listOf(
                    "path = \".\"\nholds = [\"out\", \"a.txt\"]",
                    "path = \".\"\nholds = [\"a.txt\"]",
                    "path = \"out\"\nholds = [\"c.txt\"]",
                    "path = \"missing\"\nholds = []",
                ).joinToString("") { "[[scenario.folder]]\n$it\n" },
            )
        holding(context, held("a.txt" to "a", ".config/x" to "x", "out/c.txt" to "c")) { folder ->
            val found = rules.folders.mapNotNull { ScenarioRunner.failure(it, folder) }
            agreed(found, listOf(". holds [\"a.txt\", \"out\"], expected [\"a.txt\"]", "missing: cannot read the folder"), "the failures")
        }
    }

private val comparisonFileChecks =
    Case("a comparison's file must be there and hold something") { context ->
        val rules =
            scenarioWith(
                INFO,
                listOf("full.bin", "seq", "missing.bin", "empty.bin", "hollow")
                    .joinToString("") { "[[scenario.compare]]\nkind = \"identical\"\nactual = \"$it\"\nreference = \"$it\"\n" },
            )
        holding(context, held("full.bin" to "x", "seq/a.png" to "x", "empty.bin" to "")) { folder ->
            File(folder, "hollow").mkdirs()
            val found = rules.compares.flatMap { ScenarioRunner.failures(it, folder) }
            agreed(found, listOf("missing.bin: the step wrote no such file", "empty.bin: is empty", "hollow: is empty"), "the failures")
        }
    }

private val straightPngChecks =
    Case("a PNG is written and read as the straight pixels it holds, a translucent one included, and a deeper one is refused") { context ->
        val shades = listOf(listOf(255, 128, 64, 255), listOf(10, 20, 30, 255), listOf(1, 2, 3, 255), listOf(200, 100, 50, 255))
        holding(context, emptyMap()) { folder ->
            val opaque = straightRaster(pngFile("opaque.png", folder, 2, 2) { shades[it] })
            agreed(listOf(opaque.width, opaque.height), listOf(2, 2), "the opaque PNG's size")
            agreed(opaque.pixels.map { it.toInt() and 0xff }, shades.flatten(), "the opaque PNG's pixels")
            // A translucent pixel keeps its straight levels: a decoder or an encoder that premultiplied would scale them by the alpha.
            val translucentShades = shades.mapIndexed { index, shade -> if (index == 0) listOf(255, 128, 64, 128) else shade }
            val translucent = straightRaster(pngFile("translucent.png", folder, 2, 2) { translucentShades[it] })
            agreed(translucent.pixels.map { it.toInt() and 0xff }, translucentShades.flatten(), "the translucent PNG's pixels")
            // Sixteen bits a channel, as the platform's encoder writes a bitmap of half floats.
            val deep = File(folder, "deep.png")
            val sixteen = Bitmap.createBitmap(2, 1, Bitmap.Config.RGBA_F16)
            try {
                deep.outputStream().use { require(sixteen.compress(Bitmap.CompressFormat.PNG, 100, it)) { "the deep PNG was not written" } }
            } finally {
                sixteen.recycle()
            }
            val deepRefusal =
                try {
                    straightRaster(deep)
                    null
                } catch (unreadable: RunnerFailure) {
                    unreadable.description
                }
            require(deepRefusal?.startsWith("cannot read ${deep.path} as straight 8-bit RGBA: RGBA_F16") == true) {
                "a PNG of sixteen bits a channel was answered ${deepRefusal ?: "with its pixels"}"
            }
            val absent = File(folder, "absent.png")
            val refused =
                try {
                    straightRaster(absent)
                    null
                } catch (unreadable: RunnerFailure) {
                    unreadable.description
                }
            agreed(refused, "cannot read ${absent.path}", "the PNG that is not there")
        }
    }

private val toleranceChecks =
    Case("pixels pass within the tolerance and name what is over it") { context ->
        holding(context, emptyMap()) { folder ->
            val flat = { level: Int -> { _: Int -> listOf(level, level, level, 255) } }
            val reference = pngFile("reference.png", folder, 10, 10, flat(100))
            val within = { levels: Int, fraction: Double, cap: Int -> Tolerance(levels, fraction, cap) }
            agreed(pixelFailures(pngFile("same.png", folder, 10, 10, flat(100)), reference, within(0, 0.0, 0)), emptyList(), "an equal PNG")
            val oneOff = pngFile("one-off.png", folder, 10, 10, flat(101))
            agreed(pixelFailures(oneOff, reference, within(1, 0.0, 1)), emptyList(), "a PNG one level off, where a level is allowed")
            agreed(
                pixelFailures(oneOff, reference, within(0, 0.0, 0)),
                listOf(
                    "100 of 100 pixels are more than 0 levels from the reference's: 100.000% where 0.000% is allowed",
                    "a pixel is 1 levels from the reference's, past the 0 allowed",
                ),
                "a PNG one level off, where none is",
            )
            // Three pixels in a hundred are four levels out: the fraction allows them, and the cap sets how far.
            val flips = pngFile("flips.png", folder, 10, 10) { if (it < 3) listOf(104, 100, 100, 255) else listOf(100, 100, 100, 255) }
            agreed(pixelFailures(flips, reference, within(1, 0.03, 4)), emptyList(), "three flips of four levels, allowed")
            agreed(
                pixelFailures(flips, reference, within(1, 0.02, 4)),
                listOf("3 of 100 pixels are more than 1 levels from the reference's: 3.000% where 2.000% is allowed"),
                "three flips, two percent allowed",
            )
            agreed(
                pixelFailures(flips, reference, within(1, 0.03, 3)),
                listOf("a pixel is 4 levels from the reference's, past the 3 allowed"),
                "three flips past the cap",
            )
            agreed(
                pixelFailures(pngFile("wide.png", folder, 12, 10, flat(100)), reference, within(9, 1.0, 9)),
                listOf("size 12x10 is not the reference's 10x10"),
                "a PNG of another size",
            )
        }
    }

private val cpuReferenceChecks =
    Case("a file held to the GPU's tolerance is held to the CPU's render of the same steps") { context ->
        val tolerant = "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"frame.png\"\nreference = \"frame.png\"\ntolerance = 1\n"
        val gpu = "[scenario.step.host]\ngpu = true\n"
        require(
            ScenarioRunner.heldToTheCPU(scenarioWith(gpu + INFO, tolerant)),
        ) { "a scenario on the GPU with a tolerance is not held to the CPU" }
        require(!ScenarioRunner.heldToTheCPU(scenarioWith(INFO, tolerant))) { "a scenario that never asks for the GPU is held to the CPU" }
        val identical = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"a\"\n"
        require(!ScenarioRunner.heldToTheCPU(scenarioWith(gpu + INFO, identical))) { "a file held to no tolerance is held to the CPU" }
        holding(context, emptyMap()) { gpuFolder ->
            holding(context, emptyMap()) { cpuFolder ->
                pngFile("frame.png", gpuFolder, 4, 4) { listOf(102, 100, 100, 255) }
                pngFile("frame.png", cpuFolder, 4, 4) { listOf(100, 100, 100, 255) }
                val compare = scenarioWith(gpu + INFO, tolerant).compares.single()
                agreed(
                    ScenarioRunner.failures(compare, gpuFolder, cpuFolder),
                    listOf(
                        "frame.png against the CPU's: 16 of 16 pixels are more than 1 levels from the reference's: " +
                            "100.000% where 0.000% is allowed",
                        "frame.png against the CPU's: a pixel is 2 levels from the reference's, past the 1 allowed",
                    ),
                    "the failures",
                )
                agreed(ScenarioRunner.failures(compare, gpuFolder), emptyList(), "the failures with no CPU's render, the file being there")
            }
        }
    }

private val gpuScenarioChecks =
    Case("a scenario on the GPU is held to the same steps made on the CPU") { context ->
        val calls =
            """
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
            """.trimIndent() + "\n"
        val rule = "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"frame.png\"\nreference = \"frame.png\"\ntolerance = 1\n"
        val outcome = outcomeOf(context, scenarioWith(calls, rule))
        require(outcome.failures.isEmpty() && outcome.checks == 2 && outcome.withheld == 0) { "the outcome: ${outcome.failures}" }
        // The core's own words for an engine started on the CPU, which the reference must be.
        require(outcome.rendersOn.size == 2 && outcome.rendersOn.last() == str("the app asked for CPU-only rendering")) {
            "the scenario rendered on ${outcome.rendersOn}"
        }
        require(outcome.rendersOn.first() != outcome.rendersOn.last()) { "the scenario itself ran on the CPU: ${outcome.rendersOn}" }
    }

private val failureChecks =
    Case("a plug-in that fails is no answer unless the scenario reads its failure") { context ->
        val refused = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"nothing\"\n"
        val unread = outcomeOf(context, scenarioWith(refused))
        require(unread.failures.size == 1 && unread.failures.single().startsWith("step 0: the plug-in failed: ")) { "${unread.failures}" }
        val read =
            outcomeOf(context, scenarioWith(refused, "[[scenario.expect]]\nstep = 0\npointer = \"/kind\"\nequals = \"Unsupported\"\n"))
        require(read.failures.isEmpty() && read.steps == 1 && read.checks == 1) { "${read.failures}" }
    }

private val fileFailureChecks =
    Case("a file the scenario names that cannot be read or written is the plug-in's failure in its own words") { context ->
        val comp =
            "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"run\"\n" +
                "params = { steps = [{ command = \"comp.new\", params = { width = 8, height = 6 } }] }\n"
        val calls =
            listOf(
                comp + "[[scenario.step.call]]\nkind = \"frame\"\nformat = \"png\"\nout = \"nowhere/frame.png\"" to
                    "cannot write nowhere/frame.png: ",
                "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"replay\"\nfiles = { script = \"absent.jsonl\" }" to
                    "cannot read absent.jsonl: ",
                "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"replay\"\nparams = [1]\nfiles = { script = \"absent.jsonl\" }" to
                    "`replay`: parameters that take a file must be an object",
            )
        for ((call, words) in calls) {
            val outcome = outcomeOf(context, scenarioWith(call))
            val said = outcome.failures.firstOrNull() ?: "none"
            require(outcome.failures.size == 1 && said.startsWith("step 0: the plug-in failed: $words")) { said }
        }
    }

private val fixtureChecks =
    Case("a fixture that names the container is written with the scenario's folder in its place, and any other as it is") { context ->
        holding(context, emptyMap()) { folder ->
            val runner = ScenarioRunner(context, folder)
            runner.place(Fixture("fixtures/sam-values.json", "named/values.json", true), folder)
            runner.place(Fixture("fixtures/sam-values.json", "plain/values.json", false), folder)
            val named = File(folder, "named/values.json").readText()
            val plain = File(folder, "plain/values.json").readText()
            require("<container>" !in named && folder.path in named) { "the fixture that names the container reads $named" }
            require("<container>" in plain && folder.path !in plain) { "the fixture that does not reads $plain" }
        }
    }

private val adapterChecks =
    Case("a step renders on the GPU only when the engine holds an adapter") {
        val device = obj("gpu" to str("Adreno (TM) 840"), "project" to obj("renderer" to obj("adapter" to str("Adreno (TM) 840"))))
        val cpu = obj("gpu" to str("no adapter: none found"), "project" to obj("renderer" to obj("adapter" to JSONValue.Null)))
        agreed(ScenarioRunner.failure(device, true)?.description, null, "a step on the GPU that holds an adapter")
        agreed(ScenarioRunner.failure(cpu, false)?.description, null, "a step on the CPU that holds none")
        agreed(
            ScenarioRunner.failure(cpu, true)?.description,
            "the step needs the GPU, and the engine renders on the CPU: \"no adapter: none found\"",
            "a step on the GPU that holds none",
        )
        agreed(
            ScenarioRunner.failure(obj(), true)?.description,
            "the step needs the GPU, and the engine renders on the CPU: it says nothing of why",
            "a step on the GPU of an engine that says nothing",
        )
        agreed(
            ScenarioRunner.failure(device, false)?.description,
            "the step's render on the CPU is on the GPU, \"Adreno (TM) 840\"",
            "a step on the CPU that holds an adapter",
        )
    }

private val judgeChecks =
    Case("a stop the render queue does not say the export met is an issue and never a pass") {
        val listening = Listening(stopOf("{ cancel_at = 0.5 }"))
        val at = Report(160uL, 300uL)
        agreed(listening.judge(at, listOf(str("Unqueued"), str("UserStopped"))), null, "a stop an export met")
        agreed(
            listening.judge(at, listOf(str("Unqueued"), str("Done"))),
            "the stop came at 160 of 300 frames, and the render queue says [\"Unqueued\",\"Done\"], " +
                "with no export \"UserStopped\": every frame was rendered before the stop reached the export, " +
                "so the export won the race, which the SDK has no way to hold",
            "a stop an export had ended before",
        )
        require(listening.judge(at, emptyList()) != null) { "a queue with no item says no export met the stop" }
        agreed(
            listening.judge(null, listOf(str("UserStopped"))),
            "the call answered before its export reached 0.5 of its frames, so no stop reached it: " +
                "the export won the race, which the SDK has no way to hold",
            "a call that answered before the stop",
        )
        agreed(
            Listening(stopOf("{ cancel_at = 1 }")).judge(null, emptyList()),
            "the call answered before its export reached 1 of its frames, so no stop reached it: " +
                "the export won the race, which the SDK has no way to hold",
            "a call that answered before a stop at the whole, named as the JSON it is",
        )
        agreed(Listening(stopOf("{}")).judge(null, emptyList()), null, "a call with no fraction to stop at, which is judged by nothing")
    }

private val statusChecks =
    Case("the render queue's items are read as the status of each") {
        val list = """{"items": [{"status": "Unqueued"}, {"status": "UserStopped"}, {"status": {"Failed": "x"}}]}"""
        agreed(
            Listening.statusesOf(jsonValueOf(list)),
            listOf(str("Unqueued"), str("UserStopped"), obj("Failed" to str("x"))),
            "the statuses",
        )
        val refused =
            try {
                Listening.statusesOf(jsonValueOf("{}"))
                null
            } catch (unlisted: RunnerFailure) {
                unlisted.description
            }
        agreed(refused, "the render queue lists no items: {}", "the answer of a queue with no items")
    }

/** The checks of what the runner holds a scenario to. */
internal val scenarioRunnerChecks: List<Case> =
    listOf(
        failureDocumentCheck,
        expectationChecks,
        folderRuleCheck,
        comparisonFileChecks,
        straightPngChecks,
        toleranceChecks,
        cpuReferenceChecks,
        gpuScenarioChecks,
        failureChecks,
        fileFailureChecks,
        fixtureChecks,
        adapterChecks,
        judgeChecks,
        statusChecks,
    )
