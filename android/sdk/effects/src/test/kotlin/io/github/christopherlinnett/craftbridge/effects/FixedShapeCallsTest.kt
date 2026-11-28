// The fixed-shape calls, over a transport that records requests: what each one really asks the engine
// for, in the envelope the engine's method table takes.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.InternalErrorException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.time.Duration.Companion.microseconds

class FixedShapeCallsTest {
    @Test
    fun execCarriesItsCommandAndParams() {
        val asked = Asked()
        await { fakeEngine(asked).exec("text.set", mapOf("layer" to JSONValue.Number(3.0), "text" to "Hi".jsonValue)) }
        assertEquals(
            exec("text.set", JSONValue.Object(mapOf("layer" to JSONValue.Number(3.0), "text" to JSONValue.Text("Hi")))),
            asked.all.single().second,
        )
        assertEquals("exec", asked.all.single().first)
    }

    @Test
    fun runCarriesTheWholeListOfSteps() {
        val asked = Asked()
        await {
            fakeEngine(asked).run(
                listOf(
                    RunStep("comp.new", mapOf("name" to "Card".jsonValue)),
                    RunStep("layer.newText"),
                ),
            )
        }
        val expected =
            JSONValue.Object(
                mapOf(
                    "steps" to
                        JSONValue.Array(
                            listOf(
                                JSONValue.Object(
                                    mapOf(
                                        "command" to JSONValue.Text("comp.new"),
                                        "params" to JSONValue.Object(mapOf("name" to JSONValue.Text("Card"))),
                                    ),
                                ),
                                JSONValue.Object(
                                    mapOf(
                                        "command" to JSONValue.Text("layer.newText"),
                                        "params" to JSONValue.Object(emptyMap()),
                                    ),
                                ),
                            ),
                        ),
                ),
            )
        assertEquals("run", asked.all.single().first)
        assertEquals(expected, asked.all.single().second)
    }

    @Test
    fun propsNamesItsLayerAndLeavesOutWhatWasNotGiven() {
        val asked = Asked()
        await { fakeEngine(asked).props(layer = "BG Color".jsonValue, comp = JSONValue.Number(3.0), flat = true) }
        assertEquals("props", asked.all.single().first)
        assertEquals(
            JSONValue.Object(mapOf("comp" to JSONValue.Number(3.0), "layer" to JSONValue.Text("BG Color"), "flat" to JSONValue.Bool(true))),
            asked.all.single().second,
        )
    }

    @Test
    fun getAsksOnePathOfOneLayer() {
        val asked = Asked()
        await { fakeEngine(asked).get(layer = JSONValue.Number(2.0), path = "Position", time = 1.5) }
        assertEquals("get", asked.all.single().first)
        assertEquals(
            JSONValue.Object(
                mapOf("layer" to JSONValue.Number(2.0), "path" to JSONValue.Text("Position"), "time" to JSONValue.Number(1.5)),
            ),
            asked.all.single().second,
        )
    }

    @Test
    fun setCarriesValueOrExpressionAndNotTheOneLeftOut() {
        val asked = Asked()
        await { fakeEngine(asked).set(layer = JSONValue.Number(2.0), path = "Opacity", value = 50.0.jsonValue) }
        await { fakeEngine(asked).set(layer = JSONValue.Number(2.0), path = "Opacity", expression = "time * 10") }
        assertEquals(listOf("set", "set"), asked.all.map { it.first })
        assertEquals(
            JSONValue.Object(
                mapOf("layer" to JSONValue.Number(2.0), "path" to JSONValue.Text("Opacity"), "value" to JSONValue.Number(50.0)),
            ),
            asked.all[0].second,
        )
        assertEquals(
            JSONValue.Object(
                mapOf(
                    "layer" to JSONValue.Number(2.0),
                    "path" to JSONValue.Text("Opacity"),
                    "expression" to JSONValue.Text("time * 10"),
                ),
            ),
            asked.all[1].second,
        )
    }

    @Test
    fun timingsCarriesOnlyWhatTheCallerNamed() {
        val asked = Asked()
        await { fakeEngine(asked).timings(comp = JSONValue.Number(4.0), scale = 0.5, runs = 3u) }
        assertEquals("timings", asked.all.single().first)
        assertEquals(
            JSONValue.Object(mapOf("comp" to JSONValue.Number(4.0), "scale" to JSONValue.Number(0.5), "runs" to JSONValue.Number(3.0))),
            asked.all.single().second,
        )
    }

    @Test
    fun timingsPlayIsACountOfFramesTheEngineReadsWhole() {
        // The engine reads `play` as a whole number of consecutive frames, from 1 to 1000, and refuses any
        // other value; the text it goes out as carries it whole.
        val asked = Asked()
        await { fakeEngine(asked).timings(play = 12u) }
        val sent = asked.all.single().second
        assertEquals(JSONValue.Object(mapOf("play" to JSONValue.Number(12.0))), sent)
        assertEquals("""{"play":12}""", jsonText(sent))
    }

    @Test
    fun previewRequestCarriesOnlyWhatTheCallerNamedAndAnswersWhatTheEngineWrote() {
        // The request's text is the engine's, as it wrote it: Kotlin writes it again nowhere, so iOS, which
        // takes the same text, answers the same one, though Kotlin would write this time with an exponent.
        val text = """{"comp":"Main","maxSide":640,"time":0.0005,"transparent":true}"""
        val request =
            JSONValue.Object(
                mapOf(
                    "transparent" to JSONValue.Bool(true),
                    "maxSide" to JSONValue.Number(640.0),
                    "time" to JSONValue.Number(0.0005),
                    "comp" to JSONValue.Text("Main"),
                ),
            )
        val answer =
            JSONValue.Object(
                mapOf(
                    "contentHeight" to JSONValue.Number(720.0),
                    "request" to request,
                    "requestText" to JSONValue.Text(text),
                    "contentWidth" to JSONValue.Number(1280.0),
                ),
            )
        val asked = Asked(mapOf("previewRequest" to answer))
        val engine = fakeEngine(asked)
        val previewed =
            await {
                engine.previewRequest(
                    comp = "Main".jsonValue,
                    time = 500.microseconds,
                    width = 640,
                    height = 640,
                )
            }.getOrThrow()
        await { engine.previewRequest(width = 1, height = 2) }
        assertEquals("previewRequest", asked.all.first().first)
        assertEquals(
            JSONValue.Object(
                mapOf(
                    "comp" to JSONValue.Text("Main"),
                    "time" to JSONValue.Number(0.0005),
                    "width" to JSONValue.Number(640.0),
                    "height" to JSONValue.Number(640.0),
                ),
            ),
            asked.all.first().second,
        )
        assertEquals(JSONValue.Object(mapOf("width" to JSONValue.Number(1.0), "height" to JSONValue.Number(2.0))), asked.all.last().second)
        assertEquals(request, previewed.request)
        assertEquals(1280L, previewed.contentWidth)
        assertEquals(720L, previewed.contentHeight)
        assertEquals(text, previewed.requestText)
    }

    @Test
    fun previewRequestRefusesAnAnswerThatIsNotItsShape() {
        val request = JSONValue.Object(emptyMap())
        val text = JSONValue.Text("{}")
        for (
        answer in
        listOf(
            JSONValue.Null,
            JSONValue.Object(mapOf("request" to request, "requestText" to text, "contentWidth" to JSONValue.Number(1.0))),
            JSONValue.Object(
                mapOf(
                    "request" to JSONValue.Number(1.0),
                    "requestText" to text,
                    "contentWidth" to JSONValue.Number(1.0),
                    "contentHeight" to JSONValue.Number(2.0),
                ),
            ),
            JSONValue.Object(
                mapOf(
                    "request" to request,
                    "requestText" to text,
                    "contentWidth" to JSONValue.Number(1.5),
                    "contentHeight" to JSONValue.Number(2.0),
                ),
            ),
            JSONValue.Object(
                mapOf(
                    "request" to request,
                    "contentWidth" to JSONValue.Number(1.0),
                    "contentHeight" to JSONValue.Number(2.0),
                ),
            ),
            JSONValue.Object(
                mapOf(
                    "request" to request,
                    "requestText" to request,
                    "contentWidth" to JSONValue.Number(1.0),
                    "contentHeight" to JSONValue.Number(2.0),
                ),
            ),
        )
        ) {
            val outcome = await { fakeEngine(Asked(mapOf("previewRequest" to answer))).previewRequest(width = 4, height = 4) }
            val refused = assertFailsWith<InternalErrorException> { outcome.getOrThrow() }
            assertEquals("the engine's `previewRequest` answered a document without a request and a pixel size", refused.message)
        }
    }

    @Test
    fun replayCarriesTheScriptAndTheWayToTakeItsFailures() {
        val asked = Asked()
        await { fakeEngine(asked).replay("script text", keepGoing = true) }
        assertEquals("replay", asked.all.single().first)
        assertEquals(
            JSONValue.Object(mapOf("script" to JSONValue.Text("script text"), "keepGoing" to JSONValue.Bool(true))),
            asked.all.single().second,
        )
    }
}
