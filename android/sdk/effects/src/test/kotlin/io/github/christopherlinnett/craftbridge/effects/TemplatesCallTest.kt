// Opening a template, filling its controls and what each call really sends: over a transport that
// answers the way the engine's `template.open`, `comp.info`, `file.import` and `essential.set` do.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CommandFailedException
import io.github.christopherlinnett.craftbridge.core.InternalErrorException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs

private val openAnswer =
    JSONValue.Object(
        mapOf(
            "comp" to JSONValue.Number(3.0),
            "warnings" to JSONValue.Array(listOf(JSONValue.Text("font 'Zippy' is not on this device"))),
        ),
    )

private val layerRow = JSONValue.Object(mapOf("id" to JSONValue.Number(7.0)))

private val compAnswer = JSONValue.Object(mapOf("layers" to JSONValue.Array(listOf(layerRow))))

/** One answer that serves every `exec` of the media tests: the instance layer is there for the call
 * that resolves it, the items and errors for the call that imports. */
private fun importAnswer(
    items: List<JSONValue>,
    errors: List<JSONValue> = emptyList(),
) = JSONValue.Object(
    mapOf(
        "layers" to JSONValue.Array(listOf(layerRow)),
        "items" to JSONValue.Array(items),
        "errors" to JSONValue.Array(errors),
    ),
)

private fun essentialSet(
    control: String,
    filled: Pair<String, JSONValue>,
) = exec(
    "essential.set",
    JSONValue.Object(
        mapOf(
            "comp" to JSONValue.Number(3.0),
            "layer" to JSONValue.Number(7.0),
            "control" to JSONValue.Text(control),
            filled.first to filled.second,
        ),
    ),
)

class TemplatesCallTest {
    @Test
    fun openSendsTheTemplateAndItsValuesAndAnswersTheInstance() {
        val asked = Asked(scripted = mapOf("template.open" to openAnswer))
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate", mapOf("Name" to "Sam".jsonValue)) }.getOrThrow()
        assertEquals(
            JSONValue.Object(
                mapOf(
                    "template" to JSONValue.Text("/cards/birthday.ectemplate"),
                    "values" to JSONValue.Object(mapOf("Name" to JSONValue.Text("Sam"))),
                ),
            ),
            asked.all.single().second,
        )
        assertEquals("template.open", asked.all.single().first)
        assertEquals(JSONValue.Number(3.0), card.comp)
        assertEquals(1, card.warnings.size)
    }

    @Test
    fun anOpenThatAnswersNoCompositionIsTheEngineAnsweringBadly() {
        val asked = Asked(scripted = mapOf("template.open" to JSONValue.Object(emptyMap())))
        val outcome = await { fakeEngine(asked).templates.open("/x.ectemplate") }
        val refusal = assertIs<InternalErrorException>(outcome.exceptionOrNull())
        assertEquals("`template.open` answered no composition", refusal.detail)
        assertEquals(refusal.detail, refusal.message)
    }

    @Test
    fun settingAControlResolvesTheInstanceLayerOnceThenSetsIt() {
        val asked = Asked(scripted = mapOf("template.open" to openAnswer, "exec" to compAnswer))
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate") }.getOrThrow()
        await { card.set("Name", "Sam".jsonValue) }
        // comp.info first, then the one essential.set that fills the control.
        assertEquals(exec("comp.info", JSONValue.Object(mapOf("comp" to JSONValue.Number(3.0)))), asked.all[1].second)
        assertEquals(essentialSet("Name", "value" to JSONValue.Text("Sam")), asked.all[2].second)
        await { card.set("Name", "Wilma".jsonValue) }
        assertEquals(4, asked.all.size) // the layer is remembered: no second comp.info
        assertEquals(essentialSet("Name", "value" to JSONValue.Text("Wilma")), asked.all[3].second)
    }

    @Test
    fun settingAMediaControlImportsTheFileAndReplacesWithItsItem() {
        val asked = Asked(scripted = mapOf("template.open" to openAnswer, "exec" to importAnswer(listOf(JSONValue.Number(9.0)))))
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate") }.getOrThrow()
        await { card.set("Photo", File("/sdcard/Download/dog.mp4")) }
        assertEquals(
            exec("file.import", JSONValue.Object(mapOf("paths" to JSONValue.Array(listOf(JSONValue.Text("/sdcard/Download/dog.mp4")))))),
            asked.all[1].second,
        )
        assertEquals(essentialSet("Photo", "item" to JSONValue.Number(9.0)), asked.all.last().second)
    }

    @Test
    fun anImportThatAnswersAnErrorRefusesBeforeSettingAnything() {
        val asked =
            Asked(
                scripted =
                    mapOf(
                        "template.open" to openAnswer,
                        "exec" to importAnswer(emptyList(), listOf(JSONValue.Text("not a video"))),
                    ),
            )
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate") }.getOrThrow()
        val outcome = await { card.set("Photo", File("/sdcard/x.txt")) }
        // The engine's own template runner's words for the same failure, its error printed as the JSON it came in.
        assertRefusedInTheRunnersWords("control `Photo`: cannot import /sdcard/x.txt: \"not a video\"", outcome)
        assertEquals(2, asked.all.size) // open and the import attempt: nothing was set
    }

    @Test
    fun anImportThatAnswersNoFootageRefusesInTheRunnersWords() {
        val asked = Asked(scripted = mapOf("template.open" to openAnswer, "exec" to importAnswer(emptyList())))
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate") }.getOrThrow()
        val outcome = await { card.set("Photo", File("/sdcard/x.mp4")) }
        assertRefusedInTheRunnersWords("control `Photo`: /sdcard/x.mp4 did not import as footage", outcome)
        assertEquals(2, asked.all.size)
    }

    @Test
    fun aCompositionWithNoLayerIsTheEngineAnsweringBadly() {
        val asked =
            Asked(
                scripted =
                    mapOf(
                        "template.open" to openAnswer,
                        "exec" to JSONValue.Object(mapOf("layers" to JSONValue.Array(emptyList()))),
                    ),
            )
        val card = await { fakeEngine(asked).templates.open("/cards/birthday.ectemplate") }.getOrThrow()
        val refusal = assertIs<InternalErrorException>(await { card.set("Name", "Sam".jsonValue) }.exceptionOrNull())
        assertEquals("`comp.info`: the template composition 3 holds no instance layer", refusal.detail)
    }
}

/** A media control's import refused as `file.import`'s failure, with [words] as both its detail and its message. */
private fun assertRefusedInTheRunnersWords(
    words: String,
    outcome: Result<JSONValue>,
) {
    val refusal = assertIs<CommandFailedException>(outcome.exceptionOrNull())
    assertEquals("file.import", refusal.command)
    assertEquals(words, refusal.detail)
    assertEquals(words, refusal.message)
}
