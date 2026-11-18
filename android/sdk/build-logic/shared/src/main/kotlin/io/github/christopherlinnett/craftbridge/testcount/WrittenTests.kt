package io.github.christopherlinnett.craftbridge.testcount

import java.io.File
import java.lang.reflect.Method
import java.lang.reflect.Modifier

/**
 * One test the build compiled: the class that declares the method and the method's name. The declaring
 * class is recorded because TestNG runs an inherited test under the concrete class it instantiates,
 * while the written side knows only where the method stands — the comparison of [compareWrittenAgainstRun]
 * joins the two through the type hierarchy, so a `@Test` written on an abstract base no run reaches
 * stays unmatched and named.
 */
data class WrittenTest(
    val className: String,
    val methodName: String,
) {
    val display: String
        get() = "$className.$methodName"
}

/** What reading one compiled class produced: its tests, or why it could not be read. */
class WrittenReading(
    val tests: List<WrittenTest>,
    val unreadable: List<String>,
)

/**
 * The tests the compiled classes hold, read from the compiled classes themselves and never from a list
 * somebody keeps by hand, at the bytecode level, where kotlin-test's `Test` alias has long become the
 * TestNG annotation: each class's own methods that TestNG reads as tests ([readsAsTest]), the class
 * carrying a class-level `@Test` when it or any class above it carries one, as TestNG reads a class's
 * annotation through its superclasses.
 *
 * Classes load from [loader], whose classpath is the test task's own, without running them: reading a
 * class is not running a test. A class the loader cannot load is no silent gap — it comes back in
 * [WrittenReading.unreadable] for the caller to fail on, rather than vanishing from the count.
 */
fun readWrittenTests(
    classDirs: List<File>,
    loader: ClassLoader,
): WrittenReading {
    val unreadable = mutableListOf<String>()
    val tests = mutableListOf<WrittenTest>()
    for (name in compiledClassNames(classDirs).sorted()) {
        val read =
            try {
                // The whole read, not just the load: taking a class's methods resolves the types in their
                // signatures, and a signature naming a class the test classpath does not carry fails
                // here exactly as a load does — it is the same unreadable, told with its name.
                writtenTestsOf(loader.loadClass(name))
            } catch (failure: Throwable) {
                unreadable += "$name, which the test classpath does not make readable: $failure"
                continue
            }
        tests += read
    }
    return WrittenReading(tests.sortedWith(compareBy({ it.className }, { it.methodName })), unreadable)
}

/** The class names the compiled class files under [classDirs] spell, outer and nested alike. */
private fun compiledClassNames(classDirs: List<File>): List<String> {
    val names = mutableListOf<String>()
    for (root in classDirs.filter { it.isDirectory }) {
        val folders = ArrayDeque(listOf(root))
        while (folders.isNotEmpty()) {
            val children = folders.removeLast().listFiles() ?: continue
            for (child in children.sortedBy { it.name }) {
                when {
                    child.isDirectory -> {
                        folders.addLast(child)
                    }

                    // `module-info` and `package-info` name no class with members to read.
                    child.name.endsWith(".class") && !child.name.endsWith("-info.class") -> {
                        names +=
                            child
                                .absolutePath
                                .removePrefix(root.absolutePath)
                                .trimStart(File.separatorChar)
                                .removeSuffix(".class")
                                .replace(File.separatorChar, '.')
                    }
                }
            }
        }
    }
    return names
}

private const val TESTNG_ANNOTATIONS = "org.testng.annotations."
private const val TESTNG_TEST = TESTNG_ANNOTATIONS + "Test"
private const val TESTNG_IGNORE = TESTNG_ANNOTATIONS + "Ignore"

/**
 * TestNG's own lifecycle methods — void, public, and therefore in a class-level `@Test`'s way — which it
 * runs as configuration, never as tests: a method carrying one is no written test.
 */
private val TESTNG_CONFIGURATION =
    setOf(
        "BeforeMethod",
        "AfterMethod",
        "BeforeClass",
        "AfterClass",
        "BeforeTest",
        "AfterTest",
        "BeforeSuite",
        "AfterSuite",
        "BeforeGroups",
        "AfterGroups",
    ).map { TESTNG_ANNOTATIONS + it }.toSet()

// Annotations are matched by name, not by a loaded `Test` class: this build carries no TestNG dependency
// of its own — the one version of TestNG sits in the catalog a build of its own cannot read — and a name
// is what the bytecode of the test class carries too.
private fun Array<Annotation>.names(): List<String> = map { it.annotationClass.java.name }

private fun writtenTestsOf(loaded: Class<*>): List<WrittenTest> {
    val underClassTest = generateSequence(loaded) { it.superclass }.any { TESTNG_TEST in it.declaredAnnotations.names() }
    return loaded.declaredMethods
        .filter { readsAsTest(it, underClassTest) }
        .map { WrittenTest(loaded.name, it.name) }
}

/**
 * Whether TestNG reads [method] as a test, [underClassTest] saying whether a class-level `@Test` covers
 * the class that declares it. A method under its own `@Test` is one whatever it returns and however it is
 * visible — one `@Ignore`d or switched off included, which is a test written and never run. Under a
 * class-level `@Test`, every public method but TestNG's configuration methods is one: TestNG runs the
 * void ones and drops in silence, as it reads the class, one returning a value — a test written and never
 * run, as AGENTS §10 reads it — unless that method carries another of TestNG's annotations, a data
 * provider above all, which gives it another job there and makes it no test. A Kotlin property's public
 * getter is such a method, so a class under a class-level `@Test` keeps its properties private. Compiler-made
 * methods are no tests.
 */
fun readsAsTest(
    method: Method,
    underClassTest: Boolean,
): Boolean {
    if (method.isSynthetic) return false
    val annotations = method.annotations.names()
    if (TESTNG_TEST in annotations) return true
    if (!underClassTest || !Modifier.isPublic(method.modifiers) || annotations.any { it in TESTNG_CONFIGURATION }) return false
    return method.returnType == Void.TYPE ||
        annotations.none { it.startsWith(TESTNG_ANNOTATIONS) && it != TESTNG_IGNORE }
}
