// The generated facade, called the way an app calls it: through a typed name, over a transport that
// records what the engine was really asked. The gate compiles these tests under the same
// warnings-as-errors rules as the code they test, and runs them.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.Vec2
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.coroutines.Continuation
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/** What the engine was asked, kept for the test to read. */
private class Asked {
    val all: MutableList<Pair<String, JSONValue>> = mutableListOf()

    fun record(
        method: String,
        params: JSONValue,
    ): JSONValue {
        all.add(method to params)
        return JSONValue.Text("done")
    }
}

/** Awaiting a call that cannot suspend: every fake engine here answers on the spot, so the call has
 * finished before `startCoroutine` returns and the outcome is already here — no runner beyond the
 * coroutine machinery the standard library itself carries. */
private fun await(call: suspend () -> JSONValue): Result<JSONValue> {
    var outcome: Result<JSONValue>? = null
    call.startCoroutine(
        object : Continuation<JSONValue> {
            override val context: CoroutineContext = EmptyCoroutineContext

            override fun resumeWith(result: Result<JSONValue>) {
                outcome = result
            }
        },
    )
    return outcome ?: Result.failure(IllegalStateException("the call did not finish on the spot"))
}

private class Refused : RuntimeException("the engine refused the call")

/** The envelope the transport carries every call in: the command id and its params, in one Object. */
private fun exec(
    command: String,
    params: JSONValue,
) = JSONValue.Object(
    mapOf(
        "command" to JSONValue.Text(command),
        "params" to params,
    ),
)

class GeneratedFacadeTest {
    private fun engineFor(asked: Asked): EffectsEngine = EffectsEngine { method, params -> asked.record(method, params) }

    @Test
    fun aTypedCallSendsTheCommandWithItsKeysAsJSON() {
        val asked = Asked()
        val reply =
            await {
                engineFor(asked).layer.newText(
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
        await { engineFor(asked).layer.newText() }
        val expected = exec("layer.newText", JSONValue.Object(emptyMap()))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aTypedKeyIsNotReplacedByAnExtraOne() {
        val asked = Asked()
        await {
            engineFor(asked).layer.newText(
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
            engineFor(asked).layer.mask.motionBlur(
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
        await { engineFor(asked).footage.overlayEdit(`in` = 2.0) }
        val expected = exec("footage.overlayEdit", JSONValue.Object(mapOf("in" to JSONValue.Number(2.0))))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun aCommandAndTheNamespaceBelowItKeepTheirIdsNames() {
        val asked = Asked()
        val engine = engineFor(asked)
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
        await { engineFor(asked).view.view3d.front() }
        val expected = exec("view.3d.front", JSONValue.Object(emptyMap()))
        assertEquals(expected, asked.all.first().second)
    }

    @Test
    fun anErrorFromTheEngineComesBackAsTheCallsOwnFailure() {
        val refusing = EffectsEngine { _, _ -> throw Refused() }
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
