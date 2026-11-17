// The JSON the native boundary speaks, and the values every Craft app's calls are made of.

package io.github.christopherlinnett.craftbridge.core

/** A JSON value: what a request carries to the engine and what its reply holds. */
public sealed class JSONValue : JSONRepresentable {
    /** A JSON value is its own JSON. */
    override val jsonValue: JSONValue
        get() = this

    /** JSON's `null`. */
    public data object Null : JSONValue()

    /** A JSON boolean. */
    public data class Bool(
        public val value: Boolean,
    ) : JSONValue()

    /** A JSON number. JSON has one number type, so integers are carried as `Double`. */
    public data class Number(
        public val value: Double,
    ) : JSONValue()

    /** A JSON string. `Text`, not `String`: a nested type of that name would be the one a bare `String`
     * in this file means, and the language's string is wanted far more often than JSON's own name. */
    public data class Text(
        public val value: String,
    ) : JSONValue()

    /** A JSON array. */
    public data class Array(
        public val values: List<JSONValue>,
    ) : JSONValue()

    /** A JSON object. */
    public data class Object(
        public val entries: Map<String, JSONValue>,
    ) : JSONValue()
}

/** A JSON boolean. */
public val Boolean.jsonValue: JSONValue
    get() = JSONValue.Bool(this)

/** A JSON number. */
public val Double.jsonValue: JSONValue
    get() = JSONValue.Number(this)

/** A JSON string. */
public val String.jsonValue: JSONValue
    get() = JSONValue.Text(this)

/** A value that is passed to an engine as JSON. */
public interface JSONRepresentable {
    /** The value as the engine takes it. */
    public val jsonValue: JSONValue
}

/** A point or size in two dimensions, passed as `[x, y]`. */
public data class Vec2(
    /** The first component. */
    public val x: Double,
    /** The second component. */
    public val y: Double,
) : JSONRepresentable {
    /** The components as a JSON array. */
    override val jsonValue: JSONValue
        get() = JSONValue.Array(listOf(x.jsonValue, y.jsonValue))
}

/** A point or size in three dimensions, passed as `[x, y, z]`. */
public data class Vec3(
    /** The first component. */
    public val x: Double,
    /** The second component. */
    public val y: Double,
    /** The third component. */
    public val z: Double,
) : JSONRepresentable {
    /** The components as a JSON array. */
    override val jsonValue: JSONValue
        get() = JSONValue.Array(listOf(x.jsonValue, y.jsonValue, z.jsonValue))
}

/** A colour, passed as the engine reads it: either channels or a hex string. */
public sealed class CraftColor : JSONRepresentable {
    /** The colour as the engine takes it. */
    public abstract override val jsonValue: JSONValue

    /** Red, green and blue, as the engine reads them. */
    public data class Rgb(
        public val red: Double,
        public val green: Double,
        public val blue: Double,
    ) : CraftColor() {
        /** The channels as a JSON array. */
        override val jsonValue: JSONValue
            get() = JSONValue.Array(listOf(red.jsonValue, green.jsonValue, blue.jsonValue))
    }

    /** A hexadecimal colour such as `"#ff8800"`. */
    public data class Hex(
        public val value: String,
    ) : CraftColor() {
        /** The colour as a JSON string. */
        override val jsonValue: JSONValue
            get() = JSONValue.Text(value)
    }
}
