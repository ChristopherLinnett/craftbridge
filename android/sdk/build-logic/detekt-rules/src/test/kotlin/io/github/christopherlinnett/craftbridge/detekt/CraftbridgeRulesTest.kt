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
// are built straight from a config here. Only FileWideSuppression and ErrorSuppression read the compiler's
// analysis, so only their tests compile their snippets.

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

class ErrorSuppressionProfileTest {
    private fun section(profile: Path): Config? =
        Files
            .newBufferedReader(profile)
            .use { YamlConfig.load(it) }
            .subConfig(CraftbridgeRuleSetProvider.RULE_SET)
            .takeIf { "ErrorSuppression" in it.subConfigKeys() }
            ?.subConfig("ErrorSuppression")

    @Test
    fun `the refusal runs on all written code and on no run of the generated bindings`() {
        // The convention plugin quiets the compiler's own warning about suppressing an error for every
        // file, so this rule is the only refusal of one: it must run wherever the bindings do not. A run
        // takes its profiles in order and the last that names a rule decides it, so written.yml switches
        // it on and no other profile names it: base.yml, the bindings' whole profile, would switch it on
        // there, and one read after written.yml, as shipped.yml is, could switch it off for written code.
        val profiles = Files.list(Path.of("profiles")).use { files -> files.filter { it.toString().endsWith(".yml") }.sorted().toList() }
        assertTrue(profiles.any { it.fileName.toString() == "written.yml" }, "no written.yml among $profiles")
        for (profile in profiles) {
            val switch = section(profile)?.valueOrNull<Any>(Config.ACTIVE_KEY)
            if (profile.fileName.toString() == "written.yml") {
                assertEquals(
                    true,
                    switch,
                    "written.yml does not switch ErrorSuppression on, and nothing else refuses a suppressed compiler error",
                )
            } else {
                assertEquals(null, section(profile), "$profile names ErrorSuppression, which written.yml alone decides")
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
