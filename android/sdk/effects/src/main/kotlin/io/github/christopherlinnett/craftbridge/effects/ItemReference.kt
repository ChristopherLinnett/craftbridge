// How a command names a project item, a layer, an effect or a property.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONRepresentable
import io.github.christopherlinnett.craftbridge.core.JSONValue

/** A reference to an item by number or by name, as the engine's `id|name` keys take it. */
public sealed class ItemReference : JSONRepresentable {
    /** The reference as the engine takes it. */
    public abstract override val jsonValue: JSONValue

    /** An id or an index: the engine reads either as a number. */
    public data class Number(
        public val number: Int,
    ) : ItemReference() {
        override val jsonValue: JSONValue
            get() = JSONValue.Number(number.toDouble())
    }

    /** A name, or a uid, which the engine reads as a string. */
    public data class Name(
        public val name: String,
    ) : ItemReference() {
        override val jsonValue: JSONValue
            get() = JSONValue.Text(name)
    }
}
