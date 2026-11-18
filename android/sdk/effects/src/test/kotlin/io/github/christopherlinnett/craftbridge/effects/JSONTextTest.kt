// The JSON text and JSONValue translations: what goes out must be what the engine reads, and what
// comes back must be what the calls read, with the engine's own oddities kept whole.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.InternalErrorException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class JSONTextTest {
    @Test
    fun aWholeNestPrintsAsTheEngineReadsIt() {
        val value =
            JSONValue.Object(
                mapOf(
                    "text" to "Hello, \"world\" — ünïcode\nnewline".jsonValue,
                    "count" to 3.0.jsonValue,
                    "on" to true.jsonValue,
                    "gone" to JSONValue.Null,
                    "where" to JSONValue.Array(listOf(1.5.jsonValue, 2.5.jsonValue)),
                ),
            )
        val text = jsonText(value)
        assertEquals(value, jsonValueOf(text))
    }

    @Test
    fun aWholeNumberPrintsWholeBecauseTheEngineReadsIdsAndCountsAsIntegers() {
        // A reply hands back its ids as whole numbers and the app sends them back as numbers again;
        // a 1 printed 1.0 is no composition to the engine, which reads the id as an integer, while
        // its real-numbered parameters still read a whole number as the real it is.
        assertEquals("""{"comp":1}""", jsonText(JSONValue.Object(mapOf("comp" to 1.0.jsonValue))))
        assertEquals("""{"scale":0.5}""", jsonText(JSONValue.Object(mapOf("scale" to 0.5.jsonValue))))
    }

    @Test
    fun wordsTheEngineAnswersComeBackTyped() {
        val read = jsonValueOf("""{"a":1,"b":[true,null,"x"],"c":{"d":2.5}}""")
        val expected =
            JSONValue.Object(
                mapOf(
                    "a" to JSONValue.Number(1.0),
                    "b" to JSONValue.Array(listOf(JSONValue.Bool(true), JSONValue.Null, JSONValue.Text("x"))),
                    "c" to JSONValue.Object(mapOf("d" to JSONValue.Number(2.5))),
                ),
            )
        assertEquals(expected, read)
    }

    @Test
    fun anAnswerThatIsNotJsonIsTheEngineAnsweringBadly() {
        val refusal = assertFailsWith<InternalErrorException> { jsonValueOf("this is not json") }
        assertEquals("the engine answered text that is not JSON: this is not json", refusal.detail)
        assertEquals(refusal.detail, refusal.message)
    }

    @Test
    fun anEmptyAnswerIsNoJson() {
        // Empty text is no JSON: rather than invent a value for text that came from nowhere, the codec
        // says so in the one exception the calls are allowed to answer with.
        assertFailsWith<InternalErrorException> { jsonValueOf("") }
    }
}
