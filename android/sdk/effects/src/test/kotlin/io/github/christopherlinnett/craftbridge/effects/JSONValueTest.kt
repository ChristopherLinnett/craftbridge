// The values a typed call is made of, each as the JSON the engine takes.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.CraftColor
import io.github.christopherlinnett.craftbridge.core.JSONRepresentable
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.Vec2
import io.github.christopherlinnett.craftbridge.core.Vec3
import io.github.christopherlinnett.craftbridge.core.jsonValue
import kotlin.test.Test
import kotlin.test.assertEquals

class JSONValueTest {
    private fun numbers(vararg values: Double) = JSONValue.Array(values.map { JSONValue.Number(it) })

    @Test
    fun theTypedValuesAreTheJSONTheEngineTakes() {
        assertEquals(numbers(1.0, 2.0), Vec2(1.0, 2.0).jsonValue)
        assertEquals(numbers(1.0, 2.0, 3.0), Vec3(1.0, 2.0, 3.0).jsonValue)
        assertEquals(numbers(1.0, 0.5, 0.0), CraftColor.Rgb(1.0, 0.5, 0.0).jsonValue)
        assertEquals(JSONValue.Text("#ff8800"), CraftColor.Hex("#ff8800").jsonValue)
        assertEquals(JSONValue.Number(3.0), ItemReference.Number(3).jsonValue)
        assertEquals(JSONValue.Text("Text 1"), ItemReference.Name("Text 1").jsonValue)
        assertEquals(JSONValue.Bool(true), true.jsonValue)
        assertEquals(JSONValue.Number(2.5), 2.5.jsonValue)
        assertEquals(JSONValue.Text("a"), "a".jsonValue)
    }

    @Test
    fun aJSONValueIsItsOwnJSON() {
        val value: JSONRepresentable = JSONValue.Object(mapOf("a" to JSONValue.Null))
        assertEquals(value, value.jsonValue)
    }
}
