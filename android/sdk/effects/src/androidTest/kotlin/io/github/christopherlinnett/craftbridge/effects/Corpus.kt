// The conformance corpus as Kotlin reads it: `conformance/effects/corpus.toml`, the one file every runner
// reads, parsed by a TOML reader and never copied, so a scenario is written once. The model is strict, as
// the desktop runner's and the Swift one's are: a key or a kind the runner does not read is refused, never
// ignored, so a check the corpus gains cannot go unrun here.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue
import org.tomlj.Toml
import org.tomlj.TomlArray
import org.tomlj.TomlTable

/** A failure of the runner, not of the plug-in: the scenario cannot be judged. */
internal class RunnerFailure(
    val description: String,
) : Exception(description)

/** Which sides of the corpus run a scenario. */
internal enum class Sides {
    BOTH,
    ORACLE,
    PLUGIN,
    ;

    /** Whether the plug-in side, the one a phone runs, takes part. */
    val runsPlugin: Boolean get() = this != ORACLE
}

/** A file a scenario starts with. */
internal data class Fixture(
    /** Relative to the corpus file's folder. */
    val from: String,
    /** Relative to the scenario's folder. */
    val to: String,
    /** The file's text says `<container>` where the scenario's folder goes. */
    val namesContainer: Boolean,
)

/** How the plug-in's app is built for one step. */
internal data class Host(
    /** The plug-in folder the app names, relative to the scenario's folder; plug-ins are off without one. */
    val plugins: String?,
    /** The app is handed the GPU. */
    val gpu: Boolean,
)

/**
 * How a call that reports progress is stopped: the listener asks to stop once this fraction of the frames
 * is done, and without one the call runs to its end.
 */
internal data class Stop(
    /** The fraction of the frames at which the listener stops the call, if it does. */
    val cancelAt: Double?,
) {
    /**
     * Whether the listener stops the call at [done] of [total] frames. A call that has not said how many
     * frames it has is not stopped, since half of nothing is no place to stop at.
     */
    fun stops(
        done: ULong,
        total: ULong,
    ): Boolean = cancelAt != null && total > 0uL && done.toDouble() >= cancelAt * total.toDouble()
}

/** A frame's format: PNG as the engine encodes it, or straight 8-bit RGBA written out as a PNG. */
internal enum class FrameFormat {
    PNG,
    RGBA8,
}

/** One call on the plug-in's app. */
internal sealed class Call {
    /**
     * A request of the engine's method table, with the files whose text it takes as parameters, and the
     * way its progress is heard when it reports any.
     */
    data class Request(
        val method: String,
        val params: JSONValue,
        val files: Map<String, String>,
        val progress: Stop?,
    ) : Call()

    /** A frame, written to [out]. */
    data class Frame(
        val params: JSONValue,
        val format: FrameFormat,
        val out: String,
    ) : Call()

    /** Every line of the step's input, each reply a document of the output. */
    data object Mcp : Call()
}

/** One step: what the oracle runs (`args` and `fails`, which a phone does not read) and the calls the plug-in makes to do the same. */
internal data class Step(
    /** Lines `mcp` answers. */
    val stdin: List<String>,
    val host: Host?,
    val calls: List<Call>,
)

/** The value at a pointer (RFC 6901) in the JSON document a step printed equals [equals], or holds [length] items. */
internal data class Expect(
    val step: Int,
    /** Which document of the step's output, from 0: `mcp` prints one per reply. */
    val output: Int,
    val pointer: String,
    val equals: JSONValue?,
    val length: Int?,
)

/** The two sides' output of a step compared: the oracle's, so a phone has no side to compare with. */
internal data class Same(
    val step: Int,
)

/**
 * How far two PNGs' straight RGBA pixels may be apart: a pixel is over when any channel is more than
 * [levels] from the reference's, no more than [allowedFraction] of the pixels may be over, and no channel
 * may be more than [cap] from it. The defaults are the corpus's, as the desktop runner reads them.
 */
internal data class Tolerance(
    val levels: Int,
    val allowedFraction: Double,
    val cap: Int,
)

/** A comparison of two files the steps wrote: `reference` the oracle's, [actual] the plug-in's. */
internal data class Compare(
    val actual: String,
    /**
     * The two PNGs' pixels held to a tolerance, the compositor's rule for a render on the GPU against the
     * CPU's, or null for any other kind of comparison.
     */
    val pixels: Tolerance?,
)

/** The names a folder holds when the steps are done, and no others. */
internal data class Folder(
    /** Relative to the scenario's folder, `.` the folder itself. */
    val path: String,
    val holds: List<String>,
)

/** One scenario: its steps, and what to check after them. */
internal data class Scenario(
    /** The plan's identifier (`T5`), which every failure names. */
    val id: String,
    val title: String,
    val sides: Sides,
    /** Files put in the folder before the first step. */
    val fixtures: List<Fixture>,
    val steps: List<Step>,
    /** Values the step outputs must hold. */
    val expects: List<Expect>,
    /** Outputs the two sides must print alike: the oracle's, so held on the desktop. */
    val sames: List<Same>,
    /** Files the sides must hold alike, the oracle's against the plug-in's: held on the desktop. */
    val compares: List<Compare>,
    /** The names the folders must hold. */
    val folders: List<Folder>,
) {
    /** The id and the title, which a check is reported under. */
    val checkName: String get() = "$id: $title"
}

/** The corpus: its scenarios, in the order the file has them. */
internal data class Corpus(
    val scenarios: List<Scenario>,
)

/**
 * A table of the corpus, read once every key it holds is one the runner reads: a key it does not read is a
 * check or a setting it would silently leave out.
 */
private class Table(
    value: JSONValue,
    known: Set<String>,
) {
    private val entries: Map<String, JSONValue> =
        (value as? JSONValue.Object)?.entries ?: throw RunnerFailure("the corpus holds ${value.shown} where a table belongs")

    init {
        val unknown = entries.keys.filter { it !in known }.minOrNull()
        if (unknown != null) {
            throw RunnerFailure("the runner does not read the key `$unknown`")
        }
    }

    fun value(key: String): JSONValue? = entries[key]

    fun required(key: String): JSONValue = entries[key] ?: throw RunnerFailure("the corpus lacks the key `$key`")

    fun text(key: String): String = textOrNull(key) ?: throw RunnerFailure("the corpus lacks the key `$key`")

    fun textOrNull(key: String): String? =
        when (val value = entries[key]) {
            null -> null
            is JSONValue.Text -> value.value
            else -> throw RunnerFailure("the key `$key` holds ${value.shown}, not a string")
        }

    fun number(key: String): Double? =
        when (val value = entries[key]) {
            null -> null
            is JSONValue.Number -> value.value
            else -> throw RunnerFailure("the key `$key` holds ${value.shown}, not a number")
        }

    fun int(key: String): Int? = number(key)?.let { whole(key, it) }

    /**
     * A channel's level, 0 to 255, as the desktop and Swift runners read a tolerance and a cap: a tolerance
     * past the last level would let every pixel through.
     */
    fun level(key: String): Int? =
        int(key)?.also { level ->
            if (level !in 0..MAX_LEVEL) {
                throw RunnerFailure("the key `$key` holds $level, not a level from 0 to $MAX_LEVEL")
            }
        }

    /** The entries of the table at [key], none when the key is not there. */
    fun entriesOf(key: String): Map<String, JSONValue> =
        when (val value = entries[key]) {
            null -> emptyMap()
            is JSONValue.Object -> value.entries
            else -> throw RunnerFailure("the key `$key` holds ${value.shown}, not a table")
        }

    fun flag(key: String): Boolean? =
        when (val value = entries[key]) {
            null -> null
            is JSONValue.Bool -> value.value
            else -> throw RunnerFailure("the key `$key` holds ${value.shown}, not a boolean")
        }

    fun texts(key: String): List<String>? =
        when (val value = entries[key]) {
            null -> {
                null
            }

            is JSONValue.Array -> {
                value.values.map { (it as? JSONValue.Text)?.value ?: throw RunnerFailure("the key `$key` holds ${it.shown}, not a string") }
            }

            else -> {
                throw RunnerFailure("the key `$key` holds ${value.shown}, not an array")
            }
        }

    fun <T> tables(
        key: String,
        read: (JSONValue) -> T,
    ): List<T> =
        when (val value = entries[key]) {
            null -> emptyList()
            is JSONValue.Array -> value.values.map(read)
            else -> throw RunnerFailure("the key `$key` holds ${value.shown}, not an array of tables")
        }
}

/** The highest level of an 8-bit channel. */
private const val MAX_LEVEL = 255

private fun whole(
    key: String,
    number: Double,
): Int = number.toInt().takeIf { it.toDouble() == number } ?: throw RunnerFailure("the key `$key` holds $number, not a whole number")

/**
 * The kind a table names and the table read with the keys that kind reads. The keys any kind reads are
 * checked first, so the kind itself can be read, and a kind the runner does not read is refused by name.
 */
private fun tableOf(
    value: JSONValue,
    kinds: Map<String, Set<String>>,
): Pair<String, Table> {
    val kind = Table(value, kinds.values.flatten().toSet()).text("kind")
    val known = kinds[kind] ?: throw RunnerFailure("the runner does not read the kind `$kind`")
    return kind to Table(value, known)
}

private fun decodeHost(value: JSONValue): Host {
    val table = Table(value, setOf("plugins", "gpu"))
    return Host(table.textOrNull("plugins"), table.flag("gpu") ?: false)
}

private fun decodeStop(value: JSONValue): Stop = Stop(Table(value, setOf("cancel_at")).number("cancel_at"))

private fun decodeCall(value: JSONValue): Call {
    val (kind, table) =
        tableOf(
            value,
            mapOf(
                "request" to setOf("kind", "method", "params", "files", "progress"),
                "frame" to setOf("kind", "params", "format", "out"),
                "mcp" to setOf("kind"),
            ),
        )
    val params = table.value("params") ?: JSONValue.Object(emptyMap())
    return when (kind) {
        "request" -> {
            Call.Request(
                method = table.text("method"),
                params = params,
                files =
                    table.entriesOf("files").mapValues { (name, path) ->
                        (path as? JSONValue.Text)?.value ?: throw RunnerFailure("the file `$name` holds ${path.shown}, not a path")
                    },
                progress = table.value("progress")?.let(::decodeStop),
            )
        }

        "frame" -> {
            Call.Frame(
                params = params,
                format =
                    when (val format = table.text("format")) {
                        "png" -> FrameFormat.PNG
                        "rgba8" -> FrameFormat.RGBA8
                        else -> throw RunnerFailure("the runner does not read the frame format `$format`")
                    },
                out = table.text("out"),
            )
        }

        else -> {
            Call.Mcp
        }
    }
}

private fun decodeStep(value: JSONValue): Step {
    val table = Table(value, setOf("args", "stdin", "fails", "host", "call"))
    return Step(
        stdin = table.texts("stdin").orEmpty(),
        host = table.value("host")?.let(::decodeHost),
        calls = table.tables("call", ::decodeCall),
    )
}

private fun decodeExpect(value: JSONValue): Expect {
    val table = Table(value, setOf("step", "output", "pointer", "equals", "length"))
    return Expect(
        step = table.int("step") ?: throw RunnerFailure("the corpus lacks the key `step`"),
        output = table.int("output") ?: 0,
        pointer = table.text("pointer"),
        equals = table.value("equals"),
        length = table.int("length"),
    )
}

private fun decodeSame(value: JSONValue): Same {
    val (_, table) =
        tableOf(
            value,
            mapOf(
                "output" to setOf("kind", "step", "omit", "entries", "within"),
                "timings" to setOf("kind", "step"),
            ),
        )
    return Same(table.int("step") ?: throw RunnerFailure("the corpus lacks the key `step`"))
}

private fun decodeCompare(value: JSONValue): Compare {
    val (kind, table) =
        tableOf(
            value,
            mapOf(
                "identical" to setOf("kind", "actual", "reference"),
                "json" to setOf("kind", "actual", "reference"),
                "frames" to setOf("kind", "actual", "reference", "actual_frame", "reference_frame", "exact"),
                "pixels" to setOf("kind", "actual", "reference", "tolerance", "allowed_fraction", "cap"),
            ),
        )
    val actual = table.text("actual")
    if (kind != "pixels") {
        return Compare(actual, null)
    }
    val levels = table.level("tolerance") ?: 0
    return Compare(actual, Tolerance(levels, table.number("allowed_fraction") ?: 0.0, table.level("cap") ?: levels))
}

private fun decodeFolder(value: JSONValue): Folder {
    val table = Table(value, setOf("path", "holds"))
    return Folder(table.text("path"), table.texts("holds") ?: throw RunnerFailure("the corpus lacks the key `holds`"))
}

private fun decodeFixture(value: JSONValue): Fixture {
    val table = Table(value, setOf("from", "to", "names_container"))
    return Fixture(table.text("from"), table.text("to"), table.flag("names_container") ?: false)
}

private fun decodeScenario(value: JSONValue): Scenario {
    val table = Table(value, setOf("id", "title", "sides", "fixture", "step", "expect", "same", "compare", "folder"))
    return Scenario(
        id = table.text("id"),
        title = table.text("title"),
        sides =
            when (val sides = table.textOrNull("sides") ?: "both") {
                "both" -> Sides.BOTH
                "oracle" -> Sides.ORACLE
                "plugin" -> Sides.PLUGIN
                else -> throw RunnerFailure("the runner does not read the sides `$sides`")
            },
        fixtures = table.tables("fixture", ::decodeFixture),
        steps = table.tables("step", ::decodeStep),
        expects = table.tables("expect", ::decodeExpect),
        sames = table.tables("same", ::decodeSame),
        compares = table.tables("compare", ::decodeCompare),
        folders = table.tables("folder", ::decodeFolder),
    )
}

/** The JSON value a TOML value holds. A date or time, which JSON has no value for, is refused, the corpus having none. */
private fun jsonOf(value: Any?): JSONValue =
    when (value) {
        is Boolean -> JSONValue.Bool(value)
        is Long -> JSONValue.Number(value.toDouble())
        is Double -> JSONValue.Number(value)
        is String -> JSONValue.Text(value)
        is TomlArray -> JSONValue.Array(value.toList().map(::jsonOf))
        is TomlTable -> jsonOf(value.toMap())
        is List<*> -> JSONValue.Array(value.map(::jsonOf))
        is Map<*, *> -> JSONValue.Object(value.entries.associate { (key, entry) -> key.toString() to jsonOf(entry) })
        else -> throw RunnerFailure("the corpus holds a TOML value JSON has none for: $value")
    }

/** The TOML document [text] holds, as the JSON value the model reads. */
internal fun tomlDocument(text: String): JSONValue {
    val parsed = Toml.parse(text)
    if (parsed.hasErrors()) {
        throw RunnerFailure("the corpus is not TOML: ${parsed.errors().joinToString("; ") { it.toString() }}")
    }
    return jsonOf(parsed.toMap())
}

/** The corpus [text] holds, parsed as TOML. */
internal fun corpusOf(text: String): Corpus {
    val table = Table(tomlDocument(text), setOf("scenario"))
    return Corpus(table.tables("scenario", ::decodeScenario))
}

/** The JSON text of a value, which is how a failure names it. */
internal val JSONValue.shown: String get() = jsonText(this)

/** The value at [pointer] (RFC 6901), if the document holds one. */
internal fun JSONValue.valueAt(pointer: String): JSONValue? {
    if (pointer.isEmpty()) {
        return this
    }
    if (!pointer.startsWith("/")) {
        return null
    }
    var found = this
    for (token in pointer.drop(1).split("/")) {
        val key = token.replace("~1", "/").replace("~0", "~")
        found =
            when (val here = found) {
                is JSONValue.Object -> here.entries[key]
                is JSONValue.Array -> key.toIntOrNull()?.let { here.values.getOrNull(it) }
                else -> null
            } ?: return null
    }
    return found
}

/** The corpus's own scenario ids the plug-in runs, counted from the TOML document and not through the model or its filter. */
internal fun pluginScenarioIds(text: String): List<String> =
    ((tomlDocument(text) as? JSONValue.Object)?.entries?.get("scenario") as? JSONValue.Array)
        ?.values
        .orEmpty()
        .map { (it as? JSONValue.Object)?.entries.orEmpty() }
        .filter { it["sides"] != JSONValue.Text("oracle") }
        .mapNotNull { (it["id"] as? JSONValue.Text)?.value }
