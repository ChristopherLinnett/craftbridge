// The generated facade, called the way an app calls it: through a typed name, over a transport that
// records what the engine was really asked. The gate compiles these tests under the same
// warnings-as-errors rules as the code they test, and runs them.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.Vec2
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private class Refused : RuntimeException("the engine refused the call")

class GeneratedFacadeTest {
    @Test
    fun aTypedCallSendsTheCommandWithItsKeysAsJSON() {
        val asked = Asked()
        val reply =
            await {
                fakeEngine(asked).layer.newText(
                    text = "Hello".jsonValue,
                    position = Vec2(10.0, 20.0),
                    extra = mapOf("unknownKey" to true.jsonValue),
                )
            }
        assertEquals(JSONValue.Text("done"), reply.getOrNull())
        assertEquals(1, asked.all.size)
        assertEquals("exec", asked.all.first().first)
        val expected =
            exec(
                "layer.newText",
                JSONValue.Object(
                    mapOf(
                        "text" to JSONValue.Text("Hello"),
                        "position" to JSONValue.Array(listOf(JSONValue.Number(10.0), JSONValue.Number(20.0))),
                        "unknownKey" to JSONValue.Bool(true),
                    ),
                ),
            )
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aKeyLeftOutIsNotSent() {
        val asked = Asked()
        await { fakeEngine(asked).layer.newText() }
        val expected = exec("layer.newText", JSONValue.Object(emptyMap()))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aTypedKeyIsNotReplacedByAnExtraOne() {
        val asked = Asked()
        await {
            fakeEngine(asked).layer.newText(
                text = "typed".jsonValue,
                extra = mapOf("text" to "stray".jsonValue, "other" to JSONValue.Number(1.0)),
            )
        }
        val expected =
            exec(
                "layer.newText",
                JSONValue.Object(
                    mapOf(
                        "text" to JSONValue.Text("typed"),
                        "other" to JSONValue.Number(1.0),
                    ),
                ),
            )
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aNestedNamespaceIsReachedThroughItsParent() {
        val asked = Asked()
        await {
            fakeEngine(asked).layer.mask.motionBlur(
                layer = JSONValue.Number(3.0),
                mask = "Mask 1".jsonValue,
                mode = LayerMaskCommands.MotionBlurMode.sameAsLayer,
            )
        }
        val expected =
            exec(
                "layer.mask.motionBlur",
                JSONValue.Object(
                    mapOf(
                        "layer" to JSONValue.Number(3.0),
                        "mask" to JSONValue.Text("Mask 1"),
                        "mode" to JSONValue.Text("sameAsLayer"),
                    ),
                ),
            )
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aReservedWordIsAKeyAllTheSame() {
        val asked = Asked()
        await { fakeEngine(asked).footage.overlayEdit(`in` = 2.0) }
        val expected = exec("footage.overlayEdit", JSONValue.Object(mapOf("in" to JSONValue.Number(2.0))))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aCommandAndTheNamespaceBelowItKeepTheirIdsNames() {
        val asked = Asked()
        val engine = fakeEngine(asked)
        await { engine.edit.history(steps = 2.0) }
        await { engine.edit.history.list() }
        val expected =
            listOf(
                exec("edit.history", JSONValue.Object(mapOf("steps" to JSONValue.Number(2.0)))),
                exec("edit.history.list", JSONValue.Object(emptyMap())),
            )
        assertEquals(expected, asked.all.map { it.second })
    }

    @Test
    fun aNamespaceNamedFromADigitTakesItsParentsName() {
        val asked = Asked()
        await { fakeEngine(asked).view.view3d.front() }
        val expected = exec("view.3d.front", JSONValue.Object(emptyMap()))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun anErrorFromTheEngineComesBackAsTheCallsOwnFailure() {
        val refusing = EffectsEngine(fakeCore()) { _, _ -> throw Refused() }
        val outcome = await { refusing.layer.newText() }
        assertTrue(outcome.exceptionOrNull() is Refused, "$outcome")
    }

    @Test
    fun aGeneratedEnumKeyTakesAnyWordAndNeedsAnElseInAWhen() {
        // An emitted enum key is a value class with one companion value per word, not an enum class, so
        // a `when` over it is never exhaustive: the compiler demands this `else` even with every listed
        // word named, and a word the engine adds later breaks no build that compiles today. The
        // constructor makes the value of any word, listed here or written later, as an app needs for a
        // word added since.
        fun wordOf(mode: LayerMaskCommands.MotionBlurMode): String =
            when (mode) {
                LayerMaskCommands.MotionBlurMode.sameAsLayer -> "sameAsLayer"
                LayerMaskCommands.MotionBlurMode.on -> "on"
                LayerMaskCommands.MotionBlurMode.off -> "off"
                else -> mode.word
            }
        assertEquals("on", wordOf(LayerMaskCommands.MotionBlurMode.on))
        val written = LayerMaskCommands.MotionBlurMode("aWordWrittenLater")
        assertEquals("aWordWrittenLater", wordOf(written))
        assertEquals(JSONValue.Text("aWordWrittenLater"), written.jsonValue)
    }
}
