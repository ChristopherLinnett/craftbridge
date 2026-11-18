// What the runner reads of the corpus, and how: the model refuses what it does not read, as the desktop
// runner's does, so a check the corpus gains is never silently left unrun; TOML's values become the JSON
// the engine takes; and a pointer and a fraction mean what they do on the desktop.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.JSONValue

/** A value as a failure names it: a JSON value as the JSON it is, a list of them item by item. */
private fun described(value: Any?): String =
    when (value) {
        is JSONValue -> value.shown
        is List<*> -> value.joinToString(", ", "[", "]") { described(it) }
        else -> value.toString()
    }

/** Fails with what was found against what was expected, naming [what] it is. */
internal fun <T> agreed(
    found: T,
    expected: T,
    what: String,
) {
    require(found == expected) { "$what: found ${described(found)}, expected ${described(expected)}" }
}

/** A JSON object of [entries], in the order given. */
internal fun obj(vararg entries: Pair<String, JSONValue>): JSONValue = JSONValue.Object(mapOf(*entries))

/** A JSON string. */
internal fun str(value: String): JSONValue = JSONValue.Text(value)

/** A JSON number. */
internal fun num(value: Double): JSONValue = JSONValue.Number(value)

/** A corpus of one scenario and one step, with [rest] after the step's own keys. */
private fun corpusOfOneStep(
    rest: String = "",
    step: String = "",
): String =
    "[[scenario]]\nid = \"X\"\ntitle = \"x\"\n[[scenario.step]]\n$step\n[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n$rest\n"

/** Why a corpus is refused, or null when it parses. */
private fun refusal(text: String): String? =
    try {
        corpusOf(text)
        null
    } catch (refused: RunnerFailure) {
        refused.description
    }

/** The one scenario of a corpus that holds one. */
internal fun scenarioOf(text: String): Scenario = corpusOf(text).scenarios.single()

private const val UNREAD = "the runner does not read the key"

/** Every spelling of a key or a kind the runner does not read, each with the words the refusal names it in. */
private fun refusedCorpora(): List<Pair<String, String>> =
    listOf(
        corpusOfOneStep().replace("title =", "titel =") to "$UNREAD `titel`",
        corpusOfOneStep(step = "surplus = 1") to "$UNREAD `surplus`",
        corpusOfOneStep("surplus = 1") to "$UNREAD `surplus`",
        corpusOfOneStep("progress = { cancel_after = 0.5 }") to "$UNREAD `cancel_after`",
        corpusOfOneStep("[scenario.step.host]\nsurplus = 1\n") to "$UNREAD `surplus`",
        corpusOfOneStep().replace("kind = \"request\"\nmethod", "kind = \"mcp\"\nmethod") to "$UNREAD `method`",
        corpusOfOneStep().replace("method = \"info\"", "method = \"info\"\nformat = \"png\"") to "$UNREAD `format`",
        corpusOfOneStep() + "[[scenario.expect]]\nstep = 0\npointer = \"\"\nlength = 1\nsurplus = 1\n" to "$UNREAD `surplus`",
        corpusOfOneStep() + "[[scenario.folder]]\npath = \".\"\nholds = []\nsurplus = 1\n" to "$UNREAD `surplus`",
        corpusOfOneStep() + "[[scenario.fixture]]\nfrom = \"a\"\nto = \"b\"\nsurplus = 1\n" to "$UNREAD `surplus`",
        corpusOfOneStep() + "[[scenario.same]]\nkind = \"timings\"\nstep = 0\nomit = []\n" to "$UNREAD `omit`",
        corpusOfOneStep() + "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\nexact = true\n" to
            "$UNREAD `exact`",
        corpusOfOneStep().replace("kind = \"request\"", "kind = \"launch\"") to "the runner does not read the kind `launch`",
        corpusOfOneStep() + "[[scenario.compare]]\nkind = \"similar\"\nactual = \"a\"\nreference = \"b\"\n" to
            "the runner does not read the kind `similar`",
        "surplus = 1\n" + corpusOfOneStep() to "$UNREAD `surplus`",
        corpusOfOneStep().replace("title = \"x\"", "title = \"x\"\nsides = \"neither\"") to "the runner does not read the sides `neither`",
        corpusOfOneStep().replace("id = \"X\"\n", "") to "the corpus lacks the key `id`",
        corpusOfOneStep().replace("id = \"X\"", "id = 1") to "the key `id` holds 1, not a string",
        corpusOfOneStep() + "[[scenario.expect]]\nstep = 0.5\npointer = \"\"\nlength = 1\n" to
            "the key `step` holds 0.5, not a whole number",
        corpusOfOneStep("[scenario.step.host]\ngpu = \"yes\"\n") to "the key `gpu` holds \"yes\", not a boolean",
        corpusOfOneStep() + "[[scenario.folder]]\npath = \".\"\nholds = 1\n" to "the key `holds` holds 1, not an array",
        corpusOfOneStep().replace("method = \"info\"", "method = \"info\"\nfiles = { script = 1 }") to
            "the file `script` holds 1, not a path",
        corpusOfOneStep().replace("method = \"info\"", "method = \"info\"\nfiles = [\"a.jsonl\"]") to
            "the key `files` holds [\"a.jsonl\"], not a table",
        corpusOfOneStep() + "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a\"\nreference = \"a\"\ntolerance = 256\n" to
            "the key `tolerance` holds 256, not a level from 0 to 255",
        corpusOfOneStep() + "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a\"\nreference = \"a\"\ncap = -1\n" to
            "the key `cap` holds -1, not a level from 0 to 255",
        "scenario = 1\n" to "the key `scenario` holds 1, not an array of tables",
        "[[scenario]]\nid = \"X\"\ntitle = \"x\"\nstep = [1]\n" to "the corpus holds 1 where a table belongs",
        corpusOfOneStep().replace("kind = \"request\"\nmethod = \"info\"", "kind = \"frame\"\nformat = \"gif\"\nout = \"a\"") to
            "the runner does not read the frame format `gif`",
        "[[scenario\nid = \"X\"\n" to "the corpus is not TOML",
    )

/** The keys each kind of check has, written in one corpus. */
private val everyKey =
    corpusOfOneStep(
        "progress = { cancel_at = 0.5 }\nfiles = { script = \"a.jsonl\" }",
        step = "args = [\"x\"]\nfails = true\nstdin = [\"l\"]",
    ) +
        """
        [[scenario.expect]]
        step = 0
        output = 1
        pointer = "/a"
        equals = 90
        [[scenario.same]]
        kind = "output"
        step = 0
        omit = ["/a"]
        entries = [{ pointer = "", key = "id", prefix = "x", omit = ["/b"] }]
        within = [{ pointer = "/t", omit = ["/c"] }]
        [[scenario.compare]]
        kind = "pixels"
        actual = "a.png"
        reference = "b.png"
        tolerance = 1
        allowed_fraction = 0.003
        cap = 4
        [[scenario.compare]]
        kind = "frames"
        actual = "a.mp4"
        reference = "b.png"
        reference_frame = 0
        exact = true
        [[scenario.folder]]
        path = "."
        holds = ["a"]
        [[scenario.fixture]]
        from = "a"
        to = "b"
        names_container = true
        """.trimIndent() + "\n"

/** The stop a call with progress [fraction] names, which is the part after `progress = ` of the call's own line. */
internal fun stopOf(fraction: String): Stop {
    val call =
        scenarioOf(corpusOfOneStep("progress = $fraction"))
            .steps
            .single()
            .calls
            .single()
    return (call as? Call.Request)?.progress ?: throw RunnerFailure("no stop in $fraction")
}

/** The checks of what the runner reads of the corpus. */
internal val corpusModelChecks: List<Case> =
    listOf(
        Case("the runner reads a corpus of one scenario with its defaults") {
            val scenario = scenarioOf(corpusOfOneStep())
            agreed(Triple(scenario.id, scenario.title, scenario.sides), Triple("X", "x", Sides.BOTH), "the scenario")
            require(
                scenario.steps.size == 1 && scenario.expects.isEmpty() && scenario.folders.isEmpty(),
            ) { "the scenario holds more than its step" }
            val step = scenario.steps.single()
            require(step.host == null && step.stdin.isEmpty() && step.calls.size == 1) { "the step holds more than its call: $step" }
            agreed(step.calls.single(), Call.Request("info", JSONValue.Object(emptyMap()), emptyMap(), null), "the step's call")
        },
        Case("a key or a kind the runner does not read is refused, not ignored") {
            for ((text, why) in refusedCorpora()) {
                val said = refusal(text)
                require(said?.contains(why) == true) { "expected a refusal naming $why, got ${said ?: "none"} for:\n$text" }
            }
        },
        Case("the keys each kind of check has are read") {
            val scenario = scenarioOf(everyKey)
            val step = scenario.steps.single()
            agreed(step.stdin, listOf("l"), "the step's input")
            val call = step.calls.single() as Call.Request
            agreed(call.files, mapOf("script" to "a.jsonl"), "the call's files")
            agreed(call.progress, Stop(0.5), "the call's stop")
            val expect = scenario.expects.single()
            agreed(listOf(expect.step, expect.output, expect.pointer, expect.equals), listOf(0, 1, "/a", num(90.0)), "the expectation")
            agreed(scenario.sames.size, 1, "the comparisons of output")
            agreed(scenario.compares.map { it.actual }, listOf("a.png", "a.mp4"), "the files compared")
            agreed(scenario.compares.map { it.pixels }, listOf(Tolerance(1, 0.003, 4), null), "the tolerances")
            val plain =
                scenarioOf(
                    corpusOfOneStep() + "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a\"\nreference = \"a\"\ntolerance = 2\n",
                )
            agreed(plain.compares.single().pixels, Tolerance(2, 0.0, 2), "the tolerance with no cap named, which is the tolerance")
            val widest =
                scenarioOf(
                    corpusOfOneStep() +
                        "[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a\"\nreference = \"a\"\ntolerance = 255\ncap = 0\n",
                )
            agreed(widest.compares.single().pixels, Tolerance(255, 0.0, 0), "the levels at the ends of 0 to 255")
            agreed(scenario.folders.single().holds, listOf("a"), "the folder's names")
            agreed(scenario.fixtures.single().namesContainer, true, "the fixture that names the container")
        },
        Case("TOML values become the JSON the engine takes") {
            val params = "params = { a = 1, b = 0.5, c = true, d = \"t\", e = [1, \"x\"], f = { g = 2 } }"
            val call =
                scenarioOf(corpusOfOneStep(params))
                    .steps
                    .single()
                    .calls
                    .single() as Call.Request
            val expected =
                obj(
                    "a" to num(1.0),
                    "b" to num(0.5),
                    "c" to JSONValue.Bool(true),
                    "d" to str("t"),
                    "e" to JSONValue.Array(listOf(num(1.0), str("x"))),
                    "f" to obj("g" to num(2.0)),
                )
            agreed(call.params, expected, "the call's parameters")
            val date = refusal(corpusOfOneStep("params = { when = 1979-05-27 }"))
            require(
                date?.contains("the corpus holds a TOML value JSON has none for") == true,
            ) { "a date was answered ${date ?: "with no refusal"}" }
        },
        Case("a pointer names a value by RFC 6901 and a missing one is null") {
            val document =
                obj(
                    "a" to obj("b" to JSONValue.Array(listOf(num(1.0), obj("c" to str("x"))))),
                    "k/l" to num(1.0),
                    "m~n" to num(2.0),
                    "" to num(3.0),
                )
            agreed(document.valueAt(""), document, "the pointer to the whole document")
            agreed(document.valueAt("/a/b/1/c"), str("x"), "/a/b/1/c")
            agreed(document.valueAt("/a/b/0"), num(1.0), "/a/b/0")
            agreed(document.valueAt("/k~1l"), num(1.0), "/k~1l")
            agreed(document.valueAt("/m~0n"), num(2.0), "/m~0n")
            agreed(document.valueAt("/"), num(3.0), "an empty key is a key")
            for (missing in listOf("/a/b/2", "/a/b/-1", "/a/b/x", "/z", "/a/b/0/c", "a")) {
                agreed(document.valueAt(missing), null, "the value at $missing")
            }
        },
        Case("a call is stopped at the fraction of its frames and never before it has said how many it has") {
            require(!stopOf("{}").stops(100uL, 100uL)) { "a call with no fraction is stopped" }
            val half = stopOf("{ cancel_at = 0.5 }")
            require(!half.stops(49uL, 100uL) && half.stops(50uL, 100uL) && half.stops(51uL, 100uL)) { "the half is not 50 of 100" }
            require(!half.stops(0uL, 0uL)) { "a call that has not said how many frames it has is stopped" }
            val whole = stopOf("{ cancel_at = 1 }")
            require(whole.stops(10uL, 10uL) && !whole.stops(9uL, 10uL)) { "the whole is not 10 of 10" }
        },
    )
