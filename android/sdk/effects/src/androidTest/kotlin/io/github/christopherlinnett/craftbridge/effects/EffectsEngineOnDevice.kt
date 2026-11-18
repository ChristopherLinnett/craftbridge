// The plugin's own calls, run the way an app runs them: a started engine, a template built on the device
// and opened back, a text control and a photo set, a frame rendered, an export watched, the MCP line answered.

package io.github.christopherlinnett.craftbridge.effects

import android.content.Context
import io.github.christopherlinnett.craftbridge.core.CraftColor
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.PixelFormat
import java.io.File

/** The frames of the template comp the checks build: AND-1b's export scale, so the render outlasts a
 * progress beat — one that finishes inside a single beat answers no total at all, and the progress
 * assertion would be luck, not a check — yet small enough to run in seconds on the shared emulator. */
private const val CHECK_FRAMES = 300

/** How the engine's own number reads inside a reply: this change's calls hand numbers back whole. */
private fun JSONValue.numberOrNull(): Double? = (this as? JSONValue.Number)?.value

/** The `comp` or `layer` one step of `run` answered, by the key that command names it under. */
private fun stepAnswer(
    results: JSONValue,
    step: Int,
    key: String,
): Double {
    val row = (results as? JSONValue.Array)?.values?.getOrNull(step) as? JSONValue.Object ?: error("step $step answered no row: $results")
    val answer = row.entries["result"] as? JSONValue.Object ?: error("step $step answered no result: $row")
    return answer.entries[key]?.numberOrNull() ?: error("step $step answered no `$key`: $answer")
}

private fun startOn(context: Context): EffectsEngine = EffectsEngine.start(context, gpu = GpuChoice.CPU_ONLY)

/**
 * The text the instance of [card] holds in its control [name], as the engine's own `essential.instance`
 * reads it back: the one instance layer of the card's own composition, named by its place there.
 */
private suspend fun EffectsEngine.controlText(
    card: TemplateInstance,
    name: String,
): String? {
    val answer = essential.instance(layer = "#1".jsonValue, extra = mapOf("comp" to card.comp))
    val controls = ((answer as? JSONValue.Object)?.entries?.get("controls") as? JSONValue.Array)?.values.orEmpty()
    val control = controls.filterIsInstance<JSONValue.Object>().firstOrNull { it.entries["name"] == name.jsonValue }
    return when (val value = control?.entries?.get("value")) {
        is JSONValue.Text -> value.value
        is JSONValue.Object -> (value.entries["text"] as? JSONValue.Text)?.value
        else -> null
    }
}

/** The bytes a PNG starts with, which is how a check knows the frame it was handed is one. */
private val pngStart = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte())

private fun engineChecksOnDevice(): List<Case> =
    listOf(
        Case("a started engine renders a PNG frame, answers an MCP line, and answers nothing after its shutdown") { context ->
            val engine = startOn(context)
            try {
                val results =
                    engine.run(
                        listOf(
                            RunStep(
                                "comp.new",
                                mapOf(
                                    "name" to "and1c check".jsonValue,
                                    "width" to 64.0.jsonValue,
                                    "height" to 48.0.jsonValue,
                                    "duration" to 1.0.jsonValue,
                                    "frameRate" to 30.0.jsonValue,
                                ),
                            ),
                            RunStep("layer.newText", mapOf("text" to "Sam".jsonValue)),
                        ),
                    )
                stepAnswer(results, 0, "comp")
                val picture = engine.frame()
                require(picture.data.size > pngStart.size) { "the frame answered ${picture.data.size} bytes" }
                require(picture.data.copyOfRange(0, pngStart.size).contentEquals(pngStart)) { "the frame is not PNG bytes" }
                require(
                    picture.width == 64u && picture.height == 48u,
                ) { "the frame is ${picture.width}x${picture.height}, not the comp's 64x48" }
                val answer = engine.mcp("""{"jsonrpc":"2.0","id":1,"method":"tools/list"}""")
                require(answer?.contains("tools") == true) { "the MCP line answered $answer" }
            } finally {
                engine.shutDown()
            }
            val after = runCatching { engine.frame() }.exceptionOrNull()
            require(after != null) { "the shut-down engine answered a frame after all" }
        },
        Case("a template built on the device opens back, takes its control and exports its frames with progress") { context ->
            val cache = File(Folders.appDefault(context).cache)
            val template = File(cache, "and1c-check.ectemplate")
            val output = File(cache, "and1c-out.png")
            val written = {
                cache
                    .listFiles()
                    ?.filterNotNull()
                    ?.filter { file -> file.name.startsWith("and1c-out") }
                    .orEmpty()
            }
            written().forEach { it.delete() }
            template.delete()
            val engine = startOn(context)
            try {
                val results =
                    engine.run(
                        listOf(
                            RunStep(
                                "comp.new",
                                mapOf(
                                    "name" to "and1c card".jsonValue,
                                    "width" to 640.0.jsonValue,
                                    "height" to 360.0.jsonValue,
                                    "duration" to (CHECK_FRAMES / 30.0).jsonValue,
                                    "frameRate" to 30.0.jsonValue,
                                ),
                            ),
                            RunStep("layer.newText", mapOf("text" to "Hello".jsonValue)),
                        ),
                    )
                val comp = stepAnswer(results, 0, "comp").jsonValue
                val layer = stepAnswer(results, 1, "layer").jsonValue
                engine.essential.addProperty(comp = comp, layer = layer, path = "text/sourceText".jsonValue, name = "Name".jsonValue)
                engine.essential.exportTemplate(comp = comp, path = template.path.jsonValue)
                require(template.isFile) { "the template file was not written: ${template.path}" }

                val card = engine.templates.open(template.path)
                card.set("Name", "Sam".jsonValue)
                require(engine.controlText(card, "Name") == "Sam") { "the card's Name holds ${engine.controlText(card, "Name")}, not Sam" }
                // A second card becomes the active composition; a set of the first still lands on the first.
                val second = engine.templates.open(template.path)
                card.set("Name", "Ann".jsonValue)
                require(
                    engine.controlText(card, "Name") == "Ann",
                ) { "the first card's Name holds ${engine.controlText(card, "Name")}, not Ann" }
                require(
                    engine.controlText(second, "Name") == "Hello",
                ) { "the second card's Name holds ${engine.controlText(second, "Name")}, not the template's Hello" }
                val heard = mutableListOf<Pair<ULong, ULong>>()
                engine.export(
                    request = mapOf("comp" to card.comp, "output" to output.path.jsonValue, "format" to "png".jsonValue),
                    onProgress = { done, total, _ -> heard += done to total },
                )
                require(heard.isNotEmpty()) { "the template export reported no progress at all" }
                require(
                    heard.last().second == CHECK_FRAMES.toULong(),
                ) { "the last beat did not count the comp's $CHECK_FRAMES frames: $heard" }
                require(heard.all { it.second == 0uL || it.first <= it.second }) { "a beat counted more frames than its total: $heard" }
                val files = written()
                require(files.size == CHECK_FRAMES) { "the export wrote ${files.size} of $CHECK_FRAMES frames" }
                val frame = engine.frame(comp = card.comp)
                require(frame.data.size > pngStart.size) { "the card answered ${frame.data.size} bytes" }
            } finally {
                engine.shutDown()
                written().forEach { it.delete() }
                template.delete()
            }
        },
    )

/** The square, one second long, of the solid stills the media check renders and of the card it builds
 * from them; each `comp.new` makes its comp the active one, which the commands after it work in. */
private val square =
    mapOf("width" to 32.0.jsonValue, "height" to 32.0.jsonValue, "duration" to 1.0.jsonValue, "frameRate" to 30.0.jsonValue)

/** The one id an `exec` of [command] answers under [key]: a step's `comp` or `layer`, or an import's first item. */
private suspend fun EffectsEngine.idOf(
    command: String,
    params: Map<String, JSONValue>,
    key: String,
): JSONValue {
    val answer = (exec(command, params) as? JSONValue.Object)?.entries.orEmpty()
    return (answer[key] as? JSONValue.Array)?.values?.firstOrNull() ?: answer[key] ?: error("`$command` answered no `$key`: $answer")
}

/** A still of one colour, rendered by the engine and written as a PNG to [file]: a photo the app's picker could hand over. */
private suspend fun EffectsEngine.stillInto(
    file: File,
    hex: String,
) {
    val comp = idOf("comp.new", square + mapOf("name" to "and1c $hex".jsonValue), "comp")
    layer.newSolid(color = CraftColor.Hex(hex))
    file.writeBytes(frame(comp = comp, time = 0.0).data)
}

/**
 * The red, green and blue of the pixel at the middle of [comp]'s frame at [time], as the engine renders it: its
 * first frame unless told otherwise, the active comp's for no [comp] and the comp's current time for no [time].
 */
internal suspend fun EffectsEngine.middleOf(
    comp: JSONValue?,
    time: Double? = 0.0,
): List<Int> {
    val picture = frame(comp = comp, time = time, format = PixelFormat.RGBA8_STRAIGHT)
    val at = ((picture.height.toInt() / 2) * picture.width.toInt() + picture.width.toInt() / 2) * 4
    return (0 until 3).map { channel -> picture.data[at + channel].toInt() and 0xff }
}

/** §5's photo, set on a card the way an app sets it: a media control of a template given a file. */
private fun mediaChecksOnDevice(): List<Case> =
    listOf(
        Case("a card's media control takes a photo file: the file imports and the card shows it") { context ->
            val cache = File(Folders.appDefault(context).cache)
            val red = File(cache, "and1c-red.png")
            val blue = File(cache, "and1c-blue.png")
            val template = File(cache, "and1c-photo.ectemplate")
            val made = listOf(red, blue, template)
            made.forEach { it.delete() }
            val engine = startOn(context)
            try {
                engine.stillInto(red, "#ff0000")
                engine.stillInto(blue, "#0000ff")
                val comp = engine.idOf("comp.new", square + mapOf("name" to "and1c photo card".jsonValue), "comp")
                val item = engine.idOf("file.import", mapOf("paths" to JSONValue.Array(listOf(red.path.jsonValue))), "items")
                val layer = engine.idOf("layer.addItem", mapOf("item" to item), "layer")
                engine.exec("essential.addMedia", mapOf("layer" to layer, "name" to "Photo".jsonValue))
                engine.essential.exportTemplate(comp = comp, path = template.path.jsonValue)
                require(template.isFile) { "the template file was not written: ${template.path}" }

                val card = engine.templates.open(template.path)
                val before = engine.middleOf(card.comp)
                require(
                    before[0] > 200 && before[2] < 50,
                ) { "the card shows ${before.joinToString()} before its photo is set, not the template's red" }
                card.set("Photo", blue)
                val after = engine.middleOf(card.comp)
                require(
                    after[2] > 200 && after[0] < 50,
                ) { "the card shows ${after.joinToString()} once its photo is set, not the blue photo" }
            } finally {
                engine.shutDown()
                made.forEach { it.delete() }
            }
        },
    )

/** Wired into the run in `CraftBridgeInstrumentation`, after the boundary's own checks. */
internal val apiChecks: List<Case> = engineChecksOnDevice() + mediaChecksOnDevice()
