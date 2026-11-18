package io.github.christopherlinnett.craftbridge.detekt

import dev.detekt.api.Config
import dev.detekt.api.Detektion
import dev.detekt.api.RuleInstance
import dev.detekt.api.RuleSetId
import dev.detekt.api.SetupContext
import dev.detekt.api.Severity
import dev.detekt.core.config.YamlConfig
import dev.detekt.test.TestConfig
import dev.detekt.test.lint
import dev.detekt.test.lintWithContext
import dev.detekt.test.utils.compileContentForTest
import dev.detekt.test.utils.createEnvironment
import java.net.URI
import java.nio.file.Files
import java.nio.file.Path
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The rules' own tests: each banned form is shown to fire and each legal form shown to pass, as every
// rule is proven. `active` comes from the profiles the convention plugin hands detekt; these rules
// are built straight from a config here. Only FileWideSuppression, ErrorSuppression, GeneratedNameTaken
// and UnrunTest read the compiler's analysis, so only their tests compile their snippets.

private val active = TestConfig("active" to true)

private const val MODULE_PACKAGE = "io.github.christopherlinnett.craftbridge.core"

private fun assertFindings(
    expected: Int,
    findings: List<dev.detekt.api.Finding>,
    what: String = "",
) = assertEquals(expected, findings.size, "$what ${findings.joinToString { "${it.entity.location.source}: ${it.message}" }}")

class NotNullAssertionTest {
    @Test
    fun `every not-null assertion is reported whatever its operand`() {
        val findings =
            NotNullAssertion(active).lint(
                """
                fun declared(s: String?) = s!!.length
                fun platform() = System.getProperty("k")!!
                fun chained(m: Map<String, List<String>?>) = m["k"]!!.first()!!.length
                """.trimIndent(),
            )
        assertFindings(4, findings)
    }

    @Test
    fun `other uses of the exclamation mark are not`() {
        val findings = NotNullAssertion(active).lint("fun f(a: Boolean, b: Int?) = !a && b != null && b?.inc() != 1\n")
        assertFindings(0, findings)
    }
}

class ExternalOutsidePlatformFolderTest {
    // The test's virtual file sits outside this module folder, so outside any platform/ folder of it.
    private val rule = ExternalOutsidePlatformFolder(TestConfig("active" to true, "moduleDirectory" to "/x/core"))

    @Test
    fun `an external declaration is reported wherever the test places the file`() {
        val findings =
            rule.lint(
                """
                external fun bind(handle: Long)
                external val bound: Int
                val version: Int
                    external get
                class Native {
                    companion object {
                        @JvmStatic external fun load(): Int
                    }
                }
                """.trimIndent(),
            )
        assertFindings(4, findings)
    }

    @Test
    fun `a declaration without external is not reported`() {
        val findings = rule.lint("fun bind(handle: Long) = Unit\ninternal val bound = false\n")
        assertFindings(0, findings)
    }

    @Test
    fun `a run wired without the module folder fails rather than judging by guesswork`() {
        val failure = runCatching { ExternalOutsidePlatformFolder(active).lint("external fun bind(handle: Long)\n") }.exceptionOrNull()
        assertTrue(failure?.message.orEmpty().contains("moduleDirectory"), "$failure")
    }

    @Test
    fun `only a platform folder of the file's own source set counts`() {
        fun under(path: String) = underPlatformFolder("/x/core", path)
        assertTrue(under("/x/core/src/main/kotlin/platform/Jni.kt"))
        assertTrue(under("/x/core/src/test/java/io/x/platform/deep/Jni.kt"))
        assertTrue(underPlatformFolder("/x/core/", "/x/core/src/main/kotlin/platform/Jni.kt"))
        // A package below the source root may be named kotlin or java like any other word.
        assertTrue(under("/x/core/src/main/kotlin/io/x/platform/kotlin/Deep.kt"))
        assertTrue(under("/x/core/src/main/kotlin/io/x/platform/java/Deep.kt"))
        assertFalse(under("/x/core/src/main/kotlin/io/x/Jni.kt"))
        assertFalse(under("/x/core/src/main/kotlin/io/x/platform.kt"))
        // A backslash is a character of a name on this file system, never a folder.
        assertFalse(under("/x/core/src/main/kotlin/platform\\Jni.kt"))
        // Nothing above the module counts, however its folders are named.
        assertFalse(underPlatformFolder("/src/java/platform/x/core", "/src/java/platform/x/core/src/main/kotlin/io/x/Jni.kt"))
        assertFalse(underPlatformFolder("/platform/core", "/platform/core/src/main/kotlin/io/x/Jni.kt"))
        // A file of another folder, a sibling whose name only starts the same included, is outside.
        assertFalse(under("/y/other/src/main/kotlin/platform/Jni.kt"))
        assertFalse(under("/x/cores/src/main/kotlin/platform/Jni.kt"))
        assertFalse(under("/x/core/platform/Jni.kt"))
    }
}

class ModulePackageMismatchTest {
    private fun rule(vararg expected: String) = ModulePackageMismatch(TestConfig("active" to true, "expectedPackages" to expected.toList()))

    @Test
    fun `the module package and anything under it pass`() {
        for (declared in listOf(MODULE_PACKAGE, "$MODULE_PACKAGE.plugin")) {
            assertFindings(0, rule(MODULE_PACKAGE).lint("package $declared\n"), declared)
        }
        assertFindings(
            0,
            rule("io.github.christopherlinnett.craftbridge.ffi.core").lint("package io.github.christopherlinnett.craftbridge.ffi.core\n"),
        )
    }

    @Test
    fun `another package, a sibling's prefix, the bindings package in written code and no package are reported`() {
        for (code in listOf(
            "package com.example.other\n",
            "package ${MODULE_PACKAGE}s\n",
            "package io.github.christopherlinnett.craftbridge.ffi.core\n",
            "class A\n",
        )) {
            assertFindings(1, rule(MODULE_PACKAGE).lint(code), code)
        }
    }

    @Test
    fun `a run wired without packages fails rather than passing every file`() {
        val failure = runCatching { ModulePackageMismatch(active).lint("package p\n") }.exceptionOrNull()
        assertTrue(failure?.message.orEmpty().contains("expectedPackages"), "$failure")
    }
}

class FileWideSuppressionTest {
    // The rule resolves an annotation's class, so it reads each file the way a full detekt run does.
    private val environment = createEnvironment()

    private fun lint(code: String) = FileWideSuppression(active).lintWithContext(environment, code)

    @Test
    fun `a file-wide suppression is reported however it is spelled`() {
        for (code in listOf(
            "@file:Suppress(\"all\")\npackage p\n",
            "@file:Suppress(\"ktlint\")\npackage p\n",
            "@file:Suppress(\"UNCHECKED_CAST\")\npackage p\n",
            "@file:SuppressWarnings(\"all\")\npackage p\n",
            "@file:kotlin.Suppress(\"UnusedParameter\")\npackage p\n",
            "@file:java.lang.SuppressWarnings(\"x\")\npackage p\n",
            "@file:[Suppress(\"x\") JvmName(\"P\")]\npackage p\n",
            "@file:Quiet(\"DEPRECATION\")\npackage p\n\nimport kotlin.Suppress as Quiet\n",
            "@file:Hush(\"DEPRECATION\")\npackage p\n\ntypealias Hush = Suppress\n",
        )) {
            assertFindings(1, lint(code), code)
        }
    }

    @Test
    fun `a suppression on a declaration and another file annotation pass`() {
        val findings = lint("@file:JvmName(\"P\")\npackage p\n@Suppress(\"unused\") class Ignored\n")
        assertFindings(0, findings)
    }
}

class ErrorSuppressionTest {
    // The rule asks the compiler's analysis of the file, so it reads each file the way a full detekt run does.
    private val environment = createEnvironment()

    private fun lint(code: String) = ErrorSuppression(active).lintWithContext(environment, code)

    @Test
    fun `a suppression of a compiler error is reported for each error, however it is spelled and wherever it sits`() {
        for (code in listOf(
            "package p\n\n@Suppress(\"NO_EXPLICIT_VISIBILITY_IN_API_MODE\")\nfun f() = 1\n",
            "package p\n\n@kotlin.Suppress(\"unused\", \"NO_EXPLICIT_RETURN_TYPE_IN_API_MODE\")\nfun f() = 1\n",
            "package p\n\nimport kotlin.Suppress as Quiet\n\n@Quiet(\"INVISIBLE_REFERENCE\")\nfun f() = 1\n",
            "package p\n\nclass C {\n    @Suppress(\"errors\")\n    val v = 1\n}\n",
            "package p\n\nfun f(): Int {\n    @Suppress(\"UNRESOLVED_REFERENCE\")\n    val v = 1\n    return v\n}\n",
            "@file:Suppress(\"NO_EXPLICIT_VISIBILITY_IN_API_MODE\")\n\npackage p\n",
        )) {
            assertFindings(1, lint(code), code)
        }
        val both = "package p\n\n@Suppress(\"NO_EXPLICIT_VISIBILITY_IN_API_MODE\", \"NO_EXPLICIT_RETURN_TYPE_IN_API_MODE\")\nfun f() = 1\n"
        assertFindings(2, lint(both), both)
    }

    @Test
    fun `a suppression of warnings or of other tools' checks passes`() {
        // Java's annotation suppresses no compiler error at all, which the compilation then reports itself.
        for (argument in listOf("UNCHECKED_CAST", "unused", "ktlint:standard:max-line-length", "NewApi", "NotNullAssertion")) {
            for (annotation in listOf("Suppress", "SuppressWarnings")) {
                val code = "package p\n\n@$annotation(\"$argument\")\nfun f() = 1\n"
                assertFindings(0, lint(code), code)
            }
        }
        val java = "package p\n\n@SuppressWarnings(\"NO_EXPLICIT_VISIBILITY_IN_API_MODE\")\nfun f() = 1\n"
        assertFindings(0, lint(java), java)
    }
}

class ProfileActivationTest {
    private fun section(
        profile: Path,
        rule: String,
    ): Config? =
        Files
            .newBufferedReader(profile)
            .use { YamlConfig.load(it) }
            .subConfig(CraftbridgeRuleSetProvider.RULE_SET)
            .takeIf { rule in it.subConfigKeys() }
            ?.subConfig(rule)

    private val profiles: List<Path> =
        Files.list(Path.of("profiles")).use { files -> files.filter { it.toString().endsWith(".yml") }.sorted().toList() }

    @Test
    fun `every rule of the rule set is switched on by a profile`() {
        // detekt leaves off every rule no profile names, so a rule switched on with one line, whose
        // absence no module can fail for (clean code passes without it), goes unheard with that line
        // gone: this is the only check that its activation stands. The rules are the rule set's own,
        // as detekt loads them, so a rule added to it is held to this without being listed again.
        val rules =
            CraftbridgeRuleSetProvider()
                .instance()
                .rules.keys
                .map { it.value }
        assertTrue(rules.isNotEmpty(), "the rule set holds no rule")
        for (rule in rules) {
            assertTrue(
                profiles.any { section(it, rule)?.valueOrNull<Any>(Config.ACTIVE_KEY) == true },
                "no profile switches $rule on, so detekt never runs it",
            )
        }
    }

    @Test
    fun `each rule the written profile alone must decide is switched there and named nowhere else`() {
        // Two rules must be switched on by written.yml alone, each for its own reason. A run takes
        // its profiles in order and the last that names a rule decides it, so written.yml switches
        // them on and no other profile names them: base.yml, the bindings' whole profile, would
        // switch them on there too, and one read after written.yml, as shipped.yml is, could switch
        // them off for written code.
        val rules =
            linkedMapOf(
                // The convention plugin quiets the compiler's own warning about suppressing an error
                // for every file, so this rule is the only refusal of one.
                "ErrorSuppression" to "nothing else refuses a suppressed compiler error",
                // A test TestNG never runs says nothing in any report, so no test run could name it.
                "UnrunTest" to "nothing else refuses a @Test TestNG never runs",
            )
        assertTrue(profiles.any { it.fileName.toString() == "written.yml" }, "no written.yml among $profiles")
        for (rule in rules) {
            for (profile in profiles) {
                val switch = section(profile, rule.key)?.valueOrNull<Any>(Config.ACTIVE_KEY)
                if (profile.fileName.toString() == "written.yml") {
                    assertEquals(
                        true,
                        switch,
                        "written.yml does not switch ${rule.key} on, and ${rule.value}",
                    )
                } else {
                    assertEquals(null, section(profile, rule.key), "$profile names ${rule.key}, which written.yml alone decides")
                }
            }
        }
    }
}

class FileWideSuppressionBackstopTest {
    private fun backstop(
        code: String,
        active: Boolean = true,
    ): Detektion {
        val rule = RuleInstance("FileWideSuppression", RuleSetId("craftbridge"), null, "", Severity.Error, active)
        val backstop = FileWideSuppressionBackstop()
        backstop.init(
            object : SetupContext {
                override val configUris: Collection<URI> = emptyList()
                override val config: Config = Config.empty
                override val basePath: Path = Path.of("/")
                override val outputChannel: Appendable = StringBuilder()
                override val errorChannel: Appendable = StringBuilder()
                override val properties: Map<String, Any?> = emptyMap()

                override fun register(
                    key: String,
                    value: Any,
                ) = Unit
            },
        )
        val file = compileContentForTest(code)
        backstop.onProcess(file)
        return backstop.onFinish(listOf(file), Detektion(emptyList(), listOf(rule)))
    }

    @Test
    fun `a file whose own suppression silences the rule is reported under the rule's name`() {
        for (argument in listOf("all", "ALL", "detekt:all", "craftbridge", "FileWideSuppression", "craftbridge:FileWideSuppression")) {
            val issues = backstop("@file:Suppress(\"$argument\")\npackage p\n").issues
            assertEquals(listOf("FileWideSuppression"), issues.map { it.ruleInstance.id }, argument)
        }
    }

    @Test
    fun `a file the rule reads for itself, and a run that does not switch the rule on, add nothing`() {
        assertTrue(backstop("@file:Suppress(\"ktlint\")\npackage p\n").issues.isEmpty())
        assertTrue(backstop("@file:Suppress(\"all\")\npackage p\n", active = false).issues.isEmpty())
    }
}

class GateRuleSuppressionTest {
    // A run's config as detekt builds it: the rule's own entry inside its rule set, inside the whole.
    private val config: Config =
        TestConfig(
            "craftbridge" to
                mapOf(
                    "NotNullAssertion" to mapOf("active" to true),
                    "ExternalOutsidePlatformFolder" to mapOf("active" to false),
                    "GateRuleSuppression" to mapOf("active" to true),
                ),
            "style" to mapOf("ForbiddenMethodCall" to mapOf("active" to true, "aliases" to listOf("Stop"))),
        ).subConfig("craftbridge").subConfig("GateRuleSuppression")

    @Test
    fun `a suppression detekt would honour for an active rule is reported, however it is spelled`() {
        for (argument in listOf(
            "NotNullAssertion",
            "craftbridge",
            "craftbridge:NotNullAssertion",
            "detekt.craftbridge.NotNullAssertion",
            "all",
            "detekt:all",
            "style",
            "ForbiddenMethodCall",
            "Stop",
            "GateRuleSuppression",
        )) {
            for (annotation in listOf("Suppress", "SuppressWarnings")) {
                val code = "package p\n\n@$annotation(\"unused\", \"$argument\")\nfun f(s: String?) = s!!.length\n"
                assertFindings(1, GateRuleSuppression(config).lint(code), code)
            }
        }
    }

    @Test
    fun `a nested suppression is reported where it first silences a rule`() {
        val code =
            """
            package p

            class Outer {
                @Suppress("NotNullAssertion")
                class Inner {
                    @Suppress("all")
                    fun f(s: String?) = @Suppress("craftbridge") s!!
                }
            }
            """.trimIndent()
        // Inner silences NotNullAssertion; f's `all` silences the other two besides; the expression's
        // `craftbridge` silences nothing f had not already.
        val findings = GateRuleSuppression(config).lint(code)
        assertEquals(listOf(4, 6), findings.map { it.entity.location.source.line }, findings.joinToString { it.message })
    }

    @Test
    fun `a suppression naming only other tools' checks, or an inactive rule, passes`() {
        for (argument in listOf("UNCHECKED_CAST", "ktlint:standard:max-line-length", "NewApi", "ExternalOutsidePlatformFolder")) {
            val code = "package p\n\n@Suppress(\"$argument\")\nfun f() = Unit\n"
            assertFindings(0, GateRuleSuppression(config).lint(code), code)
        }
    }
}

class UnrunTestTest {
    // The rule resolves the `@Test` and `@Ignore` annotations, their arguments and the function's return
    // type, so it reads each snippet the way a full detekt run does — beside a file declaring TestNG's own
    // annotations, which kotlin-test's `Test` and `Ignore` are typealiases of.

    private val environment = createEnvironment()

    private val testng =
        """
        package org.testng.annotations

        annotation class Test(val enabled: Boolean = true, val invocationCount: Int = 1)

        annotation class Ignore(val reason: String = "")

        annotation class BeforeMethod
        """.trimIndent()

    private fun lint(code: String) = UnrunTest(active).lintWithContext(environment, code, testng)

    private fun snippet(body: String) = "package p\n\nimport org.testng.annotations.Ignore\nimport org.testng.annotations.Test\n\n$body\n"

    @Test
    fun `a test whose method is not void is reported, however the value, or its absence, appears`() {
        for (body in listOf(
            "@Test fun f(): String = \"x\"",
            "@Test fun f() = 1",
            "@Test fun f(): Int {\n    return 1\n}",
            // `Nothing` compiles to a method returning `java.lang.Void`, `Unit?` to one returning
            // `kotlin.Unit` and `suspend` to one returning `Object`: none is `void`, so TestNG drops each.
            "@Test fun f(): Nothing = error(\"x\")",
            "@Test fun f(): Unit? = null",
            "@Test suspend fun f() {}",
            // A property's accessor carries the `@Test`, through a use-site target or on the accessor itself.
            "class A {\n    @get:Test val x: Int get() = 1\n}",
            "class A {\n    val x: Int\n        @Test get() = 1\n}",
        )) {
            assertFindings(1, lint(snippet(body)), body)
        }
    }

    @Test
    fun `a test bounded to plain Unit passes, and so does any other function`() {
        for (body in listOf(
            "@Test fun f() {}",
            "@Test fun f(): Unit = println(\"x\")",
            "typealias Done = Unit\n\n@Test fun f(): Done {}",
            "class A {\n    @Test private fun f() {}\n}",
            "fun helper(): String = \"x\"",
        )) {
            assertFindings(0, lint(snippet(body)), body)
        }
    }

    @Test
    fun `an ignore over the class is reported on the class once, a class switched off passes, and a class with no test under it passes`() {
        val ignored = snippet("@Ignore\nclass A {\n    @Test fun a() {}\n    @Test fun b(): String = \"x\"\n}")
        assertFindings(1, lint(ignored), ignored)
        // Each method's own `@Test` wins over the class's switched-off `@Test`: TestNG runs the methods
        // all the same, so the shape is legal and passes.
        val switched = snippet("@Test(enabled = false)\nclass A {\n    @Test fun a() {}\n}")
        assertFindings(0, lint(switched), switched)
        val noTests = snippet("@Ignore\nclass A {\n    fun helper(): String = \"x\"\n}")
        assertFindings(0, lint(noTests), noTests)
        // TestNG reads an `@Ignore` on the class it is written over, not on a class nested in it.
        val nested = snippet("@Ignore\nclass A {\n    class B {\n        @Test fun b() {}\n    }\n}")
        assertFindings(0, lint(nested), nested)
    }

    @Test
    fun `a test over the class makes a test of every public method, run as it says`() {
        val members =
            snippet(
                """
                @Test
                class A {
                    fun runs() {}

                    fun value(): Int = 1

                    internal fun stillPublic(): String = "x"

                    val property = 1

                    suspend fun later() {}

                    private fun hidden(): Int = 1

                    @JvmField val field = 1

                    @org.testng.annotations.BeforeMethod fun configuration(): Int = 1
                }
                """.trimIndent(),
            )
        val findings = lint(members)
        assertFindings(4, findings, members)
        for (dropped in listOf("`fun value()`", "`fun stillPublic()`", "of `property`", "`fun later()`")) {
            assertTrue(findings.any { dropped in it.message }, "$dropped: ${findings.joinToString { it.message }}")
        }
        val switchedOff = snippet("@Test(enabled = false)\nclass A {\n    fun dropped() {}\n\n    @Test fun runs() {}\n}")
        assertFindings(1, lint(switchedOff), switchedOff)
        val variable = snippet("@Test\nclass A {\n    var count = 1\n}")
        assertEquals(listOf("`var count`"), lint(variable).map { it.message.substringBefore(',') }, variable)
        val ignored = snippet("@Ignore\n@Test\nclass A {\n    fun a() {}\n\n    val b = 1\n}")
        assertEquals(
            listOf("`@Ignore` over this class runs none of the 2 tests"),
            lint(ignored).map { it.message.substringBefore(" TestNG") },
            ignored,
        )
    }

    @Test
    fun `a switch TestNG honors is reported, however written and under whatever name the file gives it`() {
        for (body in listOf(
            "@Ignore\n@Test fun f() {}",
            "@Test(enabled = false) fun f() {}",
            "const val OFF = false\n\n@Test(enabled = OFF) fun f() {}",
            "@Test(enabled = !true) fun f() {}",
            "@Test(invocationCount = 0) fun f() {}",
            "@Test(invocationCount = -1) fun f() {}",
            "const val NEVER = 0\n\n@Test(invocationCount = NEVER) fun f() {}",
        )) {
            assertFindings(1, lint(snippet(body)), body)
        }
        for (code in listOf(
            "package p\n\nimport org.testng.annotations.Test as Case\n\n@Case fun f(): String = \"x\"\n",
            "package p\n\nimport org.testng.annotations.Test\n\nprivate typealias Case = Test\n\n@Case fun f() = 1\n",
        )) {
            assertFindings(1, lint(code), code)
        }
    }

    @Test
    fun `a switch TestNG reads as on passes`() {
        for (body in listOf("@Test(enabled = true) fun f() {}", "@Test(invocationCount = 1) fun f() {}")) {
            assertFindings(0, lint(snippet(body)), body)
        }
    }

    @Test
    fun `a switch on a getter's own test names the switch, not the getter's return`() {
        // The switch is read of a getter's own `@Test` as of a function's, and a getter switched off is
        // named for its switch, not for a return that never came up for TestNG. A getter of `Unit` is
        // refused as every getter is: a property's getter always returns the property, and a `Unit` one
        // compiles to a method returning the `Unit` object — no property getter compiles to a `void` one.
        val off = snippet("class A {\n    @get:Test(enabled = false)\n    val off: Unit get() = println(\"x\")\n}")
        val switched = lint(off)
        assertFindings(1, switched, off)
        assertTrue("enabled = false" in switched.single().message, switched.single().message)
        val zero = snippet("class A {\n    @get:Test(invocationCount = 0)\n    val zero: Int get() = 1\n}")
        val counted = lint(zero)
        assertFindings(1, counted, zero)
        assertTrue("invocationCount = 0" in counted.single().message, counted.single().message)
        val gone = snippet("class A {\n    @get:Ignore\n    @get:Test val gone: Unit get() = println(\"x\")\n}")
        val ignored = lint(gone)
        assertFindings(1, ignored, gone)
        assertTrue("`@Ignore`" in ignored.single().message, ignored.single().message)
        // An `@Ignore` written on the accessor, beside the `@Test` there, is the getter's own switch too.
        val onAccessor = snippet("class A {\n    val gone: Int\n        @Ignore\n        @Test\n        get() = 1\n}")
        val accessorIgnored = lint(onAccessor)
        assertFindings(1, accessorIgnored, onAccessor)
        assertTrue("`@Ignore`" in accessorIgnored.single().message, accessorIgnored.single().message)
        for (finding in switched + counted + ignored + accessorIgnored) {
            assertTrue("write the test as a function" !in finding.message, finding.message)
        }
        val unit = snippet("class A {\n    @get:Test val u: Unit get() = println(\"x\")\n}")
        val returns = lint(unit)
        assertFindings(1, returns, unit)
        assertTrue("returns the property's value" in returns.single().message, returns.single().message)
    }

    @Test
    fun `a test on the setter is TestNG's own to report, and only that method's switch silences it`() {
        // A `@Test` read on a property's setter is TestNG's test of the setter method: that method takes a
        // value TestNG cannot supply unless a data provider fills it, so the run fails the method or runs
        // it, and says so itself either way — no silence for this rule to name. The switch that brings
        // TestNG's silence back is written on that same method, and is read as the getter's is.
        val live = snippet("class A {\n    @set:Test\n    var x: Int = 0\n}")
        assertFindings(0, lint(live), live)
        val gone = snippet("class A {\n    @set:Ignore\n    @set:Test\n    var x: Int = 0\n}")
        val ignored = lint(gone)
        assertFindings(1, ignored, gone)
        assertTrue("`@Ignore`" in ignored.single().message, ignored.single().message)
        val off = snippet("class A {\n    @set:Test(enabled = false)\n    var x: Int = 0\n}")
        val switched = lint(off)
        assertFindings(1, switched, off)
        assertTrue("enabled = false" in switched.single().message, switched.single().message)
        // An `@Ignore` switches off the method it sits on and no other: crossed over, the getter's own
        // `@Test` is as live as TestNG reads it, and drops the way a getter's test always does.
        val crossed = snippet("class A {\n    @get:Test\n    @set:Ignore\n    var x: Int = 0\n}")
        val crossing = lint(crossed)
        assertFindings(1, crossing, crossed)
        assertTrue("returns the property's value" in crossing.single().message, crossing.single().message)
    }

    @Test
    fun `each accessor's own test is read apart, whichever is written first`() {
        // TestNG reads the getter's `@Test` and the setter's as two tests: a setter's written first hides
        // nothing of the getter's, which drops as a getter's test always does.
        for (body in listOf(
            "class A {\n    @set:Test\n    @get:Test\n    var x: Int = 0\n}",
            "class A {\n    var x: Int = 0\n        @Test\n        set(value) {\n            field = value\n        }\n\n" +
                "        @Test\n        get() = field\n}",
        )) {
            val findings = lint(snippet(body))
            assertFindings(1, findings, body)
            assertTrue("returns the property's value" in findings.single().message, findings.single().message)
        }
        val both = snippet("class A {\n    @set:Test(enabled = false)\n    @get:Test(invocationCount = 0)\n    var x: Int = 0\n}")
        assertEquals(
            listOf("`@Test var x` says `invocationCount = 0`", "the setter test of `var x` says `enabled = false`"),
            lint(both).map { it.message.substringBefore(':') }.sorted(),
            both,
        )
    }

    @Test
    fun `under the test over its class a member's own ignore switches nothing, and an ignored class holding tests is refused`() {
        // TestNG reads a member's `@Ignore` only beside the member's own `@Test`: under the class's, the
        // member runs, so a `Unit` one passes and a getter drops for its return, not for the `@Ignore`.
        val runs = snippet("@Test\nclass A {\n    @Ignore\n    fun runs() {}\n}")
        assertFindings(0, lint(runs), runs)
        val getter = snippet("@Test\nclass A {\n    @get:Ignore\n    val v = 1\n}")
        val dropped = lint(getter)
        assertFindings(1, dropped, getter)
        assertTrue("returns the property's value" in dropped.single().message, dropped.single().message)
        // The `@Ignore` switches off the class it sits over and no other, so a class carrying both and
        // holding no test is switched to no test: nothing runs unseen, and the class passes.
        val empty = snippet("@Ignore\n@Test\nclass A")
        assertFindings(0, lint(empty), empty)
        // Each method is a test of its own: a `var`'s getter and public setter, and each accessor's own `@Test`.
        for ((body, count) in listOf(
            "@Ignore\n@Test\nclass A {\n    var v = 1\n}" to 2,
            "@Ignore\n@Test\nclass A {\n    var v = 1\n        private set\n}" to 1,
            "@Ignore\nclass A {\n    @get:Test\n    @set:Test\n    var v = 1\n}" to 2,
        )) {
            val code = snippet(body)
            assertEquals(
                listOf("`@Ignore` over this class runs none of the $count tests"),
                lint(code).map { it.message.substringBefore(" TestNG") },
                code,
            )
        }
    }
}

class CraftbridgeConfigValidatorTest {
    private fun problems(vararg rules: Pair<String, Any>) =
        CraftbridgeConfigValidator().validate(TestConfig("craftbridge" to mapOf("active" to true, *rules))).map { it.message }

    private fun found(vararg sections: Pair<String, Any>) = CraftbridgeConfigValidator().validate(TestConfig(*sections)).map { it.message }

    @Test
    fun `every rule of the rule set, saying whether it runs, passes`() {
        val all =
            listOf(
                "NotNullAssertion",
                "ExternalOutsidePlatformFolder",
                "ModulePackageMismatch",
                "FileWideSuppression",
                "GateRuleSuppression",
                "ErrorSuppression",
                "GeneratedNameTaken",
                "UnrunTest",
            )
        assertEquals(emptyList(), problems(*all.map { it to mapOf("active" to true) }.toTypedArray()))
        assertEquals(emptyList(), problems("NotNullAssertion" to mapOf("active" to false)))
    }

    @Test
    fun `a misspelt rule, and a rule that does not say whether it runs, fail the run`() {
        val found = problems("NotNullAsertion" to mapOf("active" to true), "FileWideSuppression" to mapOf("actve" to true))
        assertEquals(2, found.size, found.joinToString())
        assertTrue(found.any { it.startsWith("craftbridge>NotNullAsertion is no rule") }, found.joinToString())
        assertTrue(found.any { it.startsWith("craftbridge>FileWideSuppression says no `active") }, found.joinToString())
    }

    @Test
    fun `a rule with no body, and the rule set switched off, fail the run`() {
        val bodiless =
            CraftbridgeConfigValidator().validate(
                TestConfig("craftbridge" to mapOf("active" to true, "NotNullAssertion" to null)),
            )
        assertEquals(1, bodiless.size, bodiless.joinToString { it.message })
        val off = CraftbridgeConfigValidator().validate(TestConfig("craftbridge" to mapOf("active" to false)))
        assertEquals(1, off.size, off.joinToString { it.message })
    }

    @Test
    fun `a switch above the rules, on any rule set or on the processors, fails the run`() {
        val style = found("style" to mapOf("active" to false, "ForbiddenMethodCall" to mapOf("active" to true)))
        assertEquals(listOf("style>active is `false`, which switches off everything under style whatever each part says"), style)
        assertEquals(1, found("processors" to mapOf("active" to false)).size)
        assertEquals(1, found("processors" to mapOf("exclude" to listOf("FileWideSuppressionBackstop"))).size)
        assertEquals(emptyList(), found("style" to mapOf("active" to true), "processors" to mapOf("exclude" to listOf("Other"))))
    }

    @Test
    fun `a rule or rule set told to read less of the code it is given fails the run`() {
        // Each filter detekt keeps a running rule from code by, on a rule and on a rule set: detekt reads
        // the first two at both levels and the last two on the rule alone, and all four are refused at both.
        val keeps = listOf("excludes", "includes", "ignoreAnnotated", "ignoreFunction").associateWith { listOf("**/x.kt") }
        val style = mapOf("active" to true, "ForbiddenMethodCall" to (mapOf("active" to true) + keeps))
        val onRule = found("style" to style)
        assertEquals(4, onRule.count { it.startsWith("style>ForbiddenMethodCall>") }, onRule.joinToString())
        val onRuleSet = found("style" to (mapOf("active" to true) + keeps))
        assertEquals(
            listOf("excludes", "ignoreAnnotated", "ignoreFunction", "includes"),
            onRuleSet.map { it.substringAfter("style>").substringBefore(' ') }.sorted(),
            onRuleSet.joinToString(),
        )
        val craftbridge = mapOf("active" to true, "NotNullAssertion" to (mapOf("active" to true) + keeps))
        val onOwnRule = found("craftbridge" to craftbridge)
        assertEquals(4, onOwnRule.count { it.startsWith("craftbridge>NotNullAssertion>") }, onOwnRule.joinToString())
        // On this build's own rule set each filter is refused once, as a filter, not again as a rule
        // section the set does not have.
        val onOwnRuleSet = found("craftbridge" to (mapOf("active" to true) + keeps))
        assertEquals(
            listOf("excludes", "ignoreAnnotated", "ignoreFunction", "includes"),
            onOwnRuleSet.map { it.substringAfter("craftbridge>").substringBefore(' ') }.sorted(),
            onOwnRuleSet.joinToString(),
        )
        assertEquals(
            listOf(
                "style>excludes is one of the filters detekt keeps a running rule from code by, which no run's " +
                    "config holds: which rules run, and over which code, is what a run's profiles name",
            ),
            found("style" to mapOf("excludes" to listOf("**/x.kt"))),
        )
        // What this build's own rules read their options from passes, and so does detekt's own `config`,
        // whose `excludes` names the paths validation leaves out and the written config pins empty.
        assertEquals(
            emptyList(),
            found(
                "config" to mapOf("validation" to true, "excludes" to emptyList<String>()),
                "craftbridge" to
                    mapOf(
                        "ModulePackageMismatch" to mapOf("active" to true, "expectedPackages" to listOf("p")),
                        "ExternalOutsidePlatformFolder" to mapOf("active" to true, "moduleDirectory" to "/tmp/x"),
                    ),
                "style" to mapOf("ForbiddenMethodCall" to mapOf("active" to true, "methods" to listOf("kotlin.TODO"))),
            ),
        )
    }
}

class GeneratedNameTakenTest {
    // The rule asks the compiler which calls a type has, so it reads each file the way a full detekt run does.
    private val environment = createEnvironment()

    /** What a generated command facade declares: a namespace's word on the type it hangs off, and a
     * namespace's type with its commands. The test engine names each such file `dependency_<n>.kt`. */
    private val facade =
        """
        package p

        val Engine.templates: TemplatesCommands get() = TemplatesCommands(this)
        val Engine.script: ScriptCommands get() = ScriptCommands(this)

        class TemplatesCommands(internal val engine: Engine) {
            fun list(vararg named: Unit, thumbnails: Boolean? = null): Int = 0
            fun delete(id: String): Int = 0
            internal fun hidden(): Int = 0
        }

        class ScriptCommands(internal val engine: Engine)
        """.trimIndent()

    private fun lint(
        code: String,
        generated: (String) -> Boolean = { "dependency_" in it },
    ) = GeneratedNameTaken(active, generated).lintWithContext(environment, code, facade)

    @Test
    fun `a call written by hand under a name the facade gives the same type is reported however it is spelt`() {
        for (code in listOf(
            // A namespace's word on the type it hangs off: a member function, which compiles beside it…
            "package p\n\nclass Engine {\n    fun script(source: String): Int = 0\n}\n",
            // …a member property, which the compiler also refuses as a shadowed extension…
            "package p\n\nclass Engine {\n    val templates: Int = 1\n}\n",
            // …and an extension function of the same type.
            "package p\n\nclass Engine\n\nfun Engine.script(source: String): Int = 0\n",
            // A command's name on a namespace's type, with other parameters, as a property, or through an alias.
            "package p\n\nclass Engine\n\nfun TemplatesCommands.list(): Int = 0\n",
            "package p\n\nclass Engine\n\nval TemplatesCommands.delete: Int get() = 0\n",
            "package p\n\nclass Engine\n\ntypealias Spelt = TemplatesCommands\n\nfun Spelt.delete(id: Int): Int = 0\n",
        )) {
            assertFindings(1, lint(code), code)
        }
    }

    @Test
    fun `a name of its own, a call kept to the module, another type and the facade itself pass`() {
        val legal =
            """
            package p

            class Engine {
                fun exec(): Int = 0

                internal fun script(id: String): Int = 0
            }

            class Other {
                fun script(): Int = 0
            }

            fun TemplatesCommands.open(): Int = 0

            private fun TemplatesCommands.list(id: Int): Int = 0

            fun TemplatesCommands.hidden(id: Int): Int = 0
            """.trimIndent()
        assertFindings(0, lint(legal), legal)
        val generated = "package p\n\nclass Engine {\n    fun script(source: String): Int = 0\n}\n"
        assertFindings(0, lint(generated) { true }, "a generated file is the generator's: $generated")
    }

    @Test
    fun `a run wired without the generated folder fails rather than judging by guesswork`() {
        val failure =
            runCatching {
                GeneratedNameTaken(
                    active,
                ).lintWithContext(environment, "package p\n\nfun f() = 1\n")
            }.exceptionOrNull()
        assertTrue(failure?.message.orEmpty().contains("generatedFolder"), "$failure")
    }

    @Test
    fun `only a file inside the generated folder is generated`() {
        assertTrue(underFolder("/x/effects/src/generated/kotlin", "/x/effects/src/generated/kotlin/io/x/TemplatesCommands.kt"))
        assertTrue(underFolder("/x/effects/src/generated/kotlin/", "/x/effects/src/generated/kotlin/io/x/TemplatesCommands.kt"))
        assertFalse(underFolder("/x/effects/src/generated/kotlin", "/x/effects/src/main/kotlin/io/x/Templates.kt"))
        // A sibling whose name only starts the same is outside.
        assertFalse(underFolder("/x/effects/src/generated/kotlin", "/x/effects/src/generated/kotlinx/io/x/T.kt"))
        assertFalse(underFolder("/x/effects/src/generated/kotlin", "/x/effects/src/generated/kotlin"))
    }
}
