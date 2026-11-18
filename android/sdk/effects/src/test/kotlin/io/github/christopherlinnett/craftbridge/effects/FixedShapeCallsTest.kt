// The fixed-shape calls, over a transport that records requests: what each one really asks the engine
// for, in the envelope the engine's method table takes.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.test.Test
import kotlin.test.assertEquals

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
