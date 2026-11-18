package io.github.christopherlinnett.craftbridge.detekt

import io.github.christopherlinnett.craftbridge.testcount.TestngAttempt
import io.github.christopherlinnett.craftbridge.testcount.WrittenTest
import io.github.christopherlinnett.craftbridge.testcount.compareWrittenAgainstRun
import io.github.christopherlinnett.craftbridge.testcount.readTestngReports
import io.github.christopherlinnett.craftbridge.testcount.readWrittenTests
import io.github.christopherlinnett.craftbridge.testcount.readsAsTest
import org.testng.annotations.BeforeMethod
import org.testng.annotations.DataProvider
import java.io.File
import java.lang.reflect.Modifier
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The written-against-run count is proven here, and partly on this folder: its reading of TestNG's own
// report files, its comparing across inheritance and data providers, and its reading of the compiled
// classes — the last against this very test folder, the one the android stage counts too. The shapes
// below are therefore only tests TestNG runs here, so the stage's own count of this folder closes over
// each of them: a never-run shape — a method-level `@Test` on an abstract base whose subclasses carry no
// TestNG annotation of their own, which Gradle never hands TestNG, or a method TestNG drops as it reads
// its class — lives in the unit cases below and in plants, never as a test here, where it would fail
// the gate itself.

@Test
class CountedClassLevelTest {
    @BeforeMethod
    fun setupIsConfigurationNotATest() {}

    // A data provider no test asks for: under the class-level annotation it is still no test, since it
    // does another job of TestNG's.
    @DataProvider(name = "unasked")
    fun rowsAreNoTest(): Array<Array<Int>> = arrayOf(arrayOf(1))

    // No `@Test` of its own: under the class-level annotation this is a test, and TestNG runs it.
    fun countedByClassLevel() {}

    // Not public, so TestNG reads no test here and the count must not write one.
    @Suppress("UnusedPrivateMember")
    private fun privateHelperIsNoTest() {}
}

// TestNG reads a class-level `@Test` through the superclasses: the one over this base makes a test of
// its own public methods, run under each subclass, and of each subclass's own public methods too.
@Test
abstract class CountedClassLevelBase {
    fun countedUnderTheSubclass() {}
}

class CountedClassLevelLeaf : CountedClassLevelBase() {
    fun countedThroughTheBase() {}
}

open class CountingHelperBase {
    // Public, but no class-level `@Test` stands over this class or above it, so TestNG reads no test
    // here, not even under the subclass below that carries one.
    fun helperOfTheTestBelow() {}
}

@Test
class CountedOverHelperBase : CountingHelperBase() {
    fun countedOwnMethod() {}
}

// Methods asked about as if a class-level `@Test` stood over their class, which carries none, so no run
// reads them as tests: the way to pin a test TestNG would drop without writing one the count must fail.
class MethodsUnderNoTest {
    fun returnsAValue(): Int = 1

    fun returnsNothing() {}

    @Suppress("UnusedPrivateMember")
    private fun hiddenAway() {}

    // The compiler makes a public `withDefault$default` beside it, which is no test of anyone's.
    fun withDefault(row: Int = 1): Int = row
}

class CountedDataProviderTest {
    @DataProvider(name = "twoRows")
    fun twoRows(): Array<Array<Int>> = arrayOf(arrayOf(1), arrayOf(2))

    // Two invocations, one test: data supplied once is run, and run once for the count.
    @Test(dataProvider = "twoRows")
    fun countedOncePerMethod(row: Int) {}
}

private fun reportWith(vararg methods: String): File {
    val dir = Files.createTempDirectory("testng-count").toFile()
    // Built by concatenation, not an indented string literal: whitespace before the `<?xml`
    // declaration is no valid document, and trimIndent cannot strip what an interpolated line
    // leaves without indentation. The answer is the report file itself, the way the verifier is
    // handed one: a file, never the folder around it.
    val report = File(dir, "testng-results.xml")
    report.writeText(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n" +
            "<testng-results ignored=\"0\" total=\"6\" passed=\"3\" failed=\"0\" skipped=\"1\">\n" +
            "<suite name=\"Gradle suite\">\n" +
            "<test name=\"Gradle test\">\n" +
            methods.joinToString("\n") + "\n" +
            "</test>\n" +
            "</suite>\n" +
            "</testng-results>\n",
    )
    return report
}

class TestngReportReadingTest {
    private val mixed =
        reportWith(
            "<class name=\"demo.Thing\">",
            "  <test-method is-config=\"true\" name=\"setup\" status=\"PASS\"/>",
            "  <test-method name=\"was\" status=\"PASS\"/>",
            "  <test-method name=\"skipped\" status=\"SKIP\">",
            "    <exception class=\"java.lang.Throwable\">",
            "      <message><![CDATA[Method demo.Thing.skipped() on instance depends on not successfully finished methods]]></message>",
            "    </exception>",
            "  </test-method>",
            "  <test-method name=\"provided\" data-provider=\"two\" status=\"PASS\"/>",
            "  <test-method name=\"provided\" data-provider=\"two\" status=\"PASS\"/>",
            "</class>",
        )

    @Test
    fun `configuration methods are no tests`() {
        assertEquals(listOf("provided", "skipped", "was"), readTestngReports(listOf(mixed)).map { it.methodName })
    }

    @Test
    fun `data supplied invocations collapse to the one method they ran`() {
        val provided = readTestngReports(listOf(mixed)).filter { it.methodName == "provided" }
        assertEquals(1, provided.size)
        assertTrue(provided.single().ran)
    }

    @Test
    fun `a method that ran once and was skipped once ran, whichever the report lists first`() {
        val skip =
            "  <test-method name=\"both\" status=\"SKIP\"><exception class=\"x\"><message>why</message></exception></test-method>"
        val ran = "  <test-method name=\"both\" status=\"PASS\"/>"
        for (order in listOf(listOf(skip, ran), listOf(ran, skip))) {
            val report = reportWith("<class name=\"demo.Twice\">", *order.toTypedArray(), "</class>")
            assertTrue(readTestngReports(listOf(report)).single().ran, "order: $order")
        }
    }

    @Test
    fun `a skip that gives its reason is kept over one that gives none, whichever comes first`() {
        val reasoned =
            "  <test-method name=\"held\" status=\"SKIP\"><exception class=\"x\"><message>why</message></exception></test-method>"
        val bare = "  <test-method name=\"held\" status=\"SKIP\"/>"
        for (order in listOf(listOf(reasoned, bare), listOf(bare, reasoned))) {
            val report = reportWith("<class name=\"demo.Held\">", *order.toTypedArray(), "</class>")
            assertEquals("why", readTestngReports(listOf(report)).single().reason, "order: $order")
        }
    }

    @Test
    fun `a skipped method did not run and carries TestNG's own reason`() {
        val skipped = readTestngReports(listOf(mixed)).single { it.methodName == "skipped" }
        assertEquals(false, skipped.ran)
        assertEquals(
            "Method demo.Thing.skipped() on instance depends on not successfully finished methods",
            skipped.reason,
        )
    }
}

private fun attempt(
    className: String,
    methodName: String,
    status: String = "PASS",
    reason: String? = null,
) = TestngAttempt(className, methodName, status, reason)

private fun written(
    className: String,
    methodName: String,
) = WrittenTest(className, methodName)

class WrittenAgainstRunComparisonTest {
    // A hierarchy the caller injects: which class the comparison may say stands under which, exactly as
    // the verifier answers it from the test run's own classes.
    private fun compare(
        written: List<WrittenTest>,
        attempts: List<TestngAttempt>,
        unreadable: List<String> = emptyList(),
        reported: Boolean = true,
        isSubtypeOf: (String, String) -> Boolean = { child, parent -> child == parent },
    ) = compareWrittenAgainstRun(written, attempts, unreadable, reported, isSubtypeOf)

    @Test
    fun `counts close when every written test stands behind a run one`() {
        val counts = compare(listOf(written("demo.A", "x")), listOf(attempt("demo.A", "x")))
        assertTrue(counts.balanced, counts.problems.joinToString())
        assertEquals(1, counts.ran)
    }

    @Test
    fun `a test TestNG skipped is named with the reason TestNG gave`() {
        val counts = compare(listOf(written("demo.A", "s")), listOf(attempt("demo.A", "s", "SKIP", "because TestNG")))
        assertEquals(0, counts.ran)
        assertContains(counts.problems.single(), "TestNG never ran demo.A.s: it skipped it: because TestNG")
    }

    @Test
    fun `a run recorded under a subclass answers for the base the test stands on`() {
        val counts =
            compare(
                listOf(written("demo.Base", "shared")),
                listOf(attempt("demo.Sub", "shared")),
                isSubtypeOf = { child, parent -> child == "demo.Sub" && parent == "demo.Base" },
            )
        assertTrue(counts.balanced, counts.problems.joinToString())
        assertEquals(1, counts.ran)
    }

    @Test
    fun `a test no one ran fails by name`() {
        val counts = compare(listOf(written("demo.Base", "shared")), emptyList())
        assertEquals(0, counts.ran)
        assertContains(counts.problems.single(), "TestNG never ran demo.Base.shared")
    }

    @Test
    fun `a run that left no report says so before naming each test`() {
        val counts = compare(listOf(written("demo.A", "x")), emptyList(), reported = false)
        assertEquals(
            listOf(
                "  the test run left no testng-results.xml, so TestNG reported running none of them",
                "  TestNG never ran demo.A.x, and its report names no attempt at it",
            ),
            counts.problems,
        )
    }

    @Test
    fun `a run test nothing compiled fails the count too`() {
        val counts = compare(emptyList(), listOf(attempt("demo.Ghost", "phantom")))
        assertContains(counts.problems.single(), "TestNG ran demo.Ghost.phantom")
    }

    @Test
    fun `a compiled class that cannot be read is named, not passed over`() {
        val counts = compare(emptyList(), emptyList(), unreadable = listOf("demo.Gone: boom"))
        assertContains(counts.problems.single(), "demo.Gone")
    }
}

class WrittenSideReadingTest {
    // The compiled classes this very test run stands in — the same folder the stage's count reads for
    // this project, found from the class the run itself loaded.
    private val ownClasses: File =
        File(
            WrittenSideReadingTest::class.java.protectionDomain.codeSource.location
                .toURI(),
        )
    private val ownLoader: ClassLoader = checkNotNull(WrittenSideReadingTest::class.java.classLoader)

    @Test
    fun `the written side reads its shapes from the compiled classes`() {
        val reading = readWrittenTests(listOf(ownClasses), ownLoader)
        assertTrue(reading.unreadable.isEmpty(), "unreadable: ${reading.unreadable}")
        val displays = reading.tests.map { it.display }
        assertContains(displays, "${CountedClassLevelTest::class.java.name}.countedByClassLevel")
        assertContains(displays, "${CountedDataProviderTest::class.java.name}.countedOncePerMethod")
    }

    @Test
    fun `the configuration method and the data provider under a class-level test are no written tests`() {
        val displays = readWrittenTests(listOf(ownClasses), ownLoader).tests.map { it.display }
        val shapes = CountedClassLevelTest::class.java.name
        assertEquals(emptyList(), displays.filter { it == "$shapes.setupIsConfigurationNotATest" || it == "$shapes.rowsAreNoTest" })
    }

    @Test
    fun `a class-level test over a base makes tests of the base's methods and of each subclass's own`() {
        val displays = readWrittenTests(listOf(ownClasses), ownLoader).tests.map { it.display }
        assertContains(displays, "${CountedClassLevelBase::class.java.name}.countedUnderTheSubclass")
        assertContains(displays, "${CountedClassLevelLeaf::class.java.name}.countedThroughTheBase")
    }

    @Test
    fun `a public method of a base no class-level test covers is no written test`() {
        val displays = readWrittenTests(listOf(ownClasses), ownLoader).tests.map { it.display }
        assertContains(displays, "${CountedOverHelperBase::class.java.name}.countedOwnMethod")
        assertEquals(emptyList(), displays.filter { it.endsWith(".helperOfTheTestBelow") })
    }

    @Test
    fun `under a class-level test a public method returning a value is a test TestNG drops, and counted`() {
        val methods = MethodsUnderNoTest::class.java
        assertTrue(readsAsTest(methods.getMethod("returnsAValue"), underClassTest = true))
        assertTrue(readsAsTest(methods.getMethod("returnsNothing"), underClassTest = true))
        assertEquals(false, readsAsTest(methods.getMethod("returnsAValue"), underClassTest = false))
    }

    @Test
    fun `under a class-level test a method TestNG cannot see or the compiler made is no written test`() {
        val methods = MethodsUnderNoTest::class.java
        assertEquals(false, readsAsTest(methods.getDeclaredMethod("hiddenAway"), underClassTest = true))
        val made = methods.declaredMethods.single { it.name == "withDefault\$default" }
        assertTrue(made.isSynthetic && Modifier.isPublic(made.modifiers), "the compiler-made method this case reads")
        assertEquals(false, readsAsTest(made, underClassTest = true))
    }
}
