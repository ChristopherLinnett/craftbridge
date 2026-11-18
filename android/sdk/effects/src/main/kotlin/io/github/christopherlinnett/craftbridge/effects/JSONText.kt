// The JSON text a request goes out as and a reply comes back in: the core asks in text and answers in
// it, and these two translate between it and JSONValue, through kotlinx's own parser and printer.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.InternalErrorException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull

/** [value] as the JSON text the core takes a request's parameters as. */
internal fun jsonText(value: JSONValue): String = elementOf(value).toString()

/** The engine's JSON text reply, as JSONValue. */
internal fun jsonValueOf(text: String): JSONValue =
    try {
        valueOf(Json.parseToJsonElement(text))
    } catch (notJson: SerializationException) {
        // The engine answers every call with JSON text or fails as an engine; text that is not can be
        // no row of the plan's table, so it is what the core says a call it can no longer answer is.
        throw engineAnsweredBadly("the engine answered text that is not JSON: ${text.take(120)}").apply { addSuppressed(notJson) }
    }

/**
 * The engine answering in a shape no call of the plan's table answers, found by this module rather than
 * answered by the engine: the core's row for a call it can no longer answer, whose words are its detail
 * alone, as the core's own `Internal` reads.
 */
internal fun engineAnsweredBadly(words: String): InternalErrorException = InternalErrorException(words, words)

private fun elementOf(value: JSONValue): JsonElement =
    when (value) {
        is JSONValue.Null -> {
            JsonNull
        }

        is JSONValue.Bool -> {
            JsonPrimitive(value.value)
        }

        // A whole number is written whole, with no decimal point: the engine reads ids and counts as
        // integers and a 1 written 1.0 is none of them, while a whole number still answers the calls
        // that read their parameter as a real one.
        is JSONValue.Number -> {
            if (value.value.rem(1.0) == 0.0 && value.value >= -9.0e18 && value.value <= 9.0e18) {
                JsonPrimitive(value.value.toLong())
            } else {
                JsonPrimitive(value.value)
            }
        }

        is JSONValue.Text -> {
            JsonPrimitive(value.value)
        }

        is JSONValue.Array -> {
            JsonArray(value.values.map(::elementOf))
        }

        is JSONValue.Object -> {
            JsonObject(value.entries.mapValues { (_, entry) -> elementOf(entry) })
        }
    }

private fun valueOf(element: JsonElement): JSONValue =
    when (element) {
        is JsonNull -> {
            JSONValue.Null
        }

        is JsonArray -> {
            JSONValue.Array(element.map(::valueOf))
        }

        is JsonObject -> {
            JSONValue.Object(element.mapValues { (_, entry) -> valueOf(entry) })
        }

        is JsonPrimitive -> {
            when {
                element.isString -> {
                    JSONValue.Text(element.content)
                }

                else -> {
                    element.booleanOrNull?.jsonBoolean()
                        ?: element.doubleOrNull?.jsonNumber()
                        ?: JSONValue.Text(element.content)
                }
            }
        }
    }

private fun Boolean.jsonBoolean(): JSONValue.Bool = JSONValue.Bool(this)

private fun Double.jsonNumber(): JSONValue.Number = JSONValue.Number(this)
