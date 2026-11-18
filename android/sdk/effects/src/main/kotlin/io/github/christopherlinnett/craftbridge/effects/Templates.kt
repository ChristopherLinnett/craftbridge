// Opening a template, setting its controls and rendering it: §5's card, and the engine calls it lives
// on. The engine answers these as methods of its own table, not as commands of the `templates`
// namespace, so the calls are extensions of that namespace's generated type, beside the commands it
// generated, under names none of them has, and not members of EffectsEngine, whose `templates` the
// namespace already is (AGENTS.md §9).

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.JSONRepresentable
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.ProgressHandler
import io.github.christopherlinnett.craftbridge.core.jsonValue
import java.io.File
import java.util.concurrent.atomic.AtomicReference

/** The engine's template methods all name the template's own path and the values to put on its controls
 * as it opens. */
private fun templateFields(
    template: String,
    values: Map<String, JSONValue>,
): Map<String, JSONValue> = mapOf("template" to JSONValue.Text(template), "values" to JSONValue.Object(values))

/**
 * Open the `.ectemplate` at [template], applying [values] to its controls as it opens, and answer the
 * [TemplateInstance] they made: its composition to preview and export, and the template's text layers
 * whose fonts this phone lacks, the warnings §5 shows beside the card.
 *
 * @throws CraftException with the engine's own answer, its refusal of a bad control value among them,
 * which it makes before it imports anything.
 */
public suspend fun TemplatesCommands.open(
    template: String,
    values: Map<String, JSONValue> = emptyMap(),
): TemplateInstance {
    val answer = engine.request("template.open", JSONValue.Object(templateFields(template, values)))
    val entries = (answer as? JSONValue.Object)?.entries.orEmpty()
    val comp = entries["comp"] as? JSONValue.Number ?: throw engineAnsweredBadly("`template.open` answered no composition")
    val warnings = (entries["warnings"] as? JSONValue.Array)?.values.orEmpty()
    return TemplateInstance(engine, comp, warnings)
}

/**
 * Export the template at [template], with [values] applied as it opens, through the engine's render
 * request [output] — the format and files the plan's table gives — and hear every progress report the
 * engine makes in [onProgress]. The coroutine's cancel token reaches the engine, so cancelling the call
 * winds the render down and clears what it had written.
 *
 * @throws CraftException with whatever the engine answers the render request.
 */
public suspend fun TemplatesCommands.render(
    template: String,
    output: Map<String, JSONValue>,
    values: Map<String, JSONValue> = emptyMap(),
    onProgress: ProgressHandler,
): JSONValue =
    engine.requestWithProgress(
        "template.render",
        JSONValue.Object(templateFields(template, values) + mapOf("output" to JSONValue.Object(output))),
        onProgress,
    )

/**
 * An opened template: its composition, and the controls to fill before previewing or exporting it.
 *
 * The engine answers an open with the composition and the warnings alone, and names a control's layer
 * by id, so the first call finds the composition's one instance layer once and the calls after it go
 * straight to one `essential.set` each.
 */
public class TemplateInstance internal constructor(
    private val engine: EffectsEngine,
    /** The composition the engine wrapped the template's instance in: what a frame, a preview and an export name. */
    public val comp: JSONValue.Number,
    /** The text layers whose fonts this phone lacks, as the engine describes them: what §5 warns about. */
    public val warnings: List<JSONValue>,
) {
    private val instanceLayer = AtomicReference<JSONValue.Number?>()

    /**
     * Set the template's control named [name] to [value], as the engine takes it for that control's kind.
     *
     * @throws CraftException when no control bears that name, or the value does not suit it.
     */
    public suspend fun set(
        name: String,
        value: JSONRepresentable,
    ): JSONValue = controlWrite(name, "value" to value.jsonValue)

    /**
     * Give the template's media control named [name] the image or video file [media]: it is imported into
     * the project first, as the engine's own template runner imports a media control's file, and then
     * replaces the control's footage. A [File], not a path's text, so a text value meant for a text control
     * can never be read as a file to import.
     *
     * @throws CraftException when the file cannot be imported or imports as no footage, in the words the
     * engine's own template runner refuses it with, or when the named control is none this card has.
     */
    public suspend fun set(
        name: String,
        media: File,
    ): JSONValue {
        val path = media.path
        val imported = engine.command("file.import", null, "paths" to JSONValue.Array(listOf(JSONValue.Text(path))))
        val entries = (imported as? JSONValue.Object)?.entries.orEmpty()
        val failure = (entries["errors"] as? JSONValue.Array)?.values?.firstOrNull()
        if (failure != null) {
            throw importRefused("control `$name`: cannot import $path: ${jsonText(failure)}")
        }
        val item =
            (entries["items"] as? JSONValue.Array)?.values?.firstOrNull()
                ?: throw importRefused("control `$name`: $path did not import as footage")
        return controlWrite(name, "item" to item)
    }

    /** The one `essential.set` a control write is: the named control of this card's instance layer,
     * taking [written] — a value, or an imported item to stand in for one. */
    private suspend fun controlWrite(
        name: String,
        written: Pair<String, JSONValue>,
    ): JSONValue = engine.command("essential.set", null, "comp" to comp, "layer" to instanceLayerOf(), "control" to name.jsonValue, written)

    /** The instance layer: found once, by the one composition a template open makes, and kept. */
    private suspend fun instanceLayerOf(): JSONValue.Number {
        instanceLayer.get()?.let { return it }
        val info = engine.command("comp.info", null, "comp" to comp)
        val layer =
            ((info as? JSONValue.Object)?.entries?.get("layers") as? JSONValue.Array)
                ?.values
                ?.filterIsInstance<JSONValue.Object>()
                ?.firstNotNullOfOrNull { row -> row.entries["id"] as? JSONValue.Number }
                ?: throw engineAnsweredBadly("`comp.info`: the template composition ${jsonText(comp)} holds no instance layer")
        instanceLayer.compareAndSet(null, layer)
        return instanceLayer.get() ?: layer
    }
}

/**
 * A media control's file the engine did not import, in the words the engine's own template runner
 * refuses one with (`craftbridge-effects`' `templates.rs`, `import_footage`), the error it was answered
 * printed as the JSON it came in, so its detail reads as the engine's own does. No engine call answers this
 * failure itself, so the detail is its message too.
 */
private fun importRefused(words: String): CommandFailedException = CommandFailedException("file.import", words, null, words)
