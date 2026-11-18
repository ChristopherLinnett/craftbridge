package io.github.christopherlinnett.craftbridge.testcount

import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.Project
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.provider.Property
import org.gradle.api.provider.Provider
import org.gradle.api.services.BuildService
import org.gradle.api.services.BuildServiceParameters
import org.gradle.api.tasks.Classpath
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.Internal
import org.gradle.api.tasks.Optional
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.api.tasks.testing.Test
import java.io.File
import java.net.URL
import java.net.URLClassLoader
import java.util.concurrent.Callable
import java.util.concurrent.ConcurrentHashMap

/** The verifier's name, the same in every project it counts the tests of. */
private const val VERIFIER = "verifyTestCounts"

/**
 * Wire the written-against-run check onto a project's test task, shared by every caller so the check
 * is one definition (AGENTS §5): `craftbridge.android.library` wires it onto every module's unit tests
 * and `:detekt-rules` wires it onto its own rules' tests.
 *
 * [install] registers the project's one verifier task and must run while Gradle still takes new tasks —
 * from the plugin's `apply` or a build file's own body — because registering a task from within the
 * configuration of another task is exactly what Gradle refuses. [verify] then wires the test task onto
 * that verifier and may run whenever the test task itself is being configured.
 *
 * The task's framework setup gains one thing: TestNG's own `XMLReporter` as its listener, writing
 * `testng-results.xml` — TestNG's own machine-readable report, whose shape its own writer keeps stable
 * — into the folder TestNG's options name as the task's output. The verifier then runs as a finalizer of
 * the test task, which Gradle runs whatever the test run answered, and fails with each test named that
 * the build compiled without TestNG reporting it run, and each run test no compiled class marks as a
 * test; when the counts close, it prints them, per module and for the rules' own tests alike.
 */
object TestCountVerification {
    private const val WIRING = "io.github.christopherlinnett.craftbridge.testcount.wiring"
    private const val REACHED = "io.github.christopherlinnett.craftbridge.testcount.reached"

    /**
     * What one project's verifier reads, shared between [install] and [verify] through the project. It is
     * filled while Gradle configures the test task, and Gradle refuses any change to the task container —
     * a registered task being configured through its provider included — while another task is being
     * configured; the verifier reads these values when it runs, so wiring a test task never returns to
     * the container.
     */
    private class Wiring(
        val testTask: Property<String>,
        val testClasses: ConfigurableFileCollection,
        val runClasspath: ConfigurableFileCollection,
        val reports: ConfigurableFileCollection,
        val reached: Provider<ReachedTestTasks>,
    )

    /** Register the project's one verifier task, once. */
    fun install(project: Project) {
        val reached = project.gradle.sharedServices.registerIfAbsent(REACHED, ReachedTestTasks::class.java) {}
        val wiring =
            Wiring(
                project.objects.property(String::class.java),
                project.objects.fileCollection(),
                project.objects.fileCollection(),
                project.objects.fileCollection(),
                reached,
            )
        project.extensions.extraProperties.set(WIRING, wiring)
        project.tasks.register(VERIFIER, VerifyWrittenAgainstRun::class.java) {
            it.group = "verification"
            it.description =
                "Compare the tests the build compiles against the tests TestNG reports it ran, and print them."
            it.moduleName.set(project.path)
            it.testTask.set(wiring.testTask)
            it.testClasses.from(wiring.testClasses)
            it.runClasspath.from(wiring.runClasspath)
            it.testngReports.from(wiring.reports)
            it.reached.set(reached)
            it.usesService(reached)
            // The counts print on every run of the gate, not only when an input moved: the stage
            // says them, every run, as a matter of record.
            it.outputs.upToDateWhen { false }
        }
    }

    /** Point the project's test task at the verifier, which [install] registered for the project. */
    fun verify(task: Test) {
        val project = task.project
        val wiring =
            project.extensions.extraProperties.get(WIRING) as? Wiring
                ?: throw GradleException("call TestCountVerification.install on ${project.path} first")
        // One verifier reads one run: a second test task's report folder, left from whenever that task
        // last ran, would vouch for tests this build never ran.
        wiring.testTask.orNull?.let { wired ->
            throw GradleException(
                "${project.path}: $VERIFIER counts one test task's run, and both $wired and ${task.path} are wired to it.",
            )
        }
        wiring.testTask.set(task.path)
        val reportsDir = project.layout.buildDirectory.dir("testng-results/" + task.name)
        task.useTestNG { options ->
            // Not Gradle's converted XML of its own running: TestNG's own report file, written by
            // TestNG's own reporter, which is what the count of run tests is taken from.
            options.listeners = setOf("org.testng.reporters.XMLReporter")
            options.outputDirectory = reportsDir.get().asFile
        }
        // The reporter writes one file per TestNG run, so the gate keeps the run to one: one worker,
        // one TestNG, one report — parallel test workers would overwrite each other's file, and a
        // report missing half the classes would fail the stage loudly rather than quietly.
        task.maxParallelForks = 1
        task.forkEvery = 0
        // The report is the test task's output, so it answers for the test classes as they stand only
        // when Gradle reached the task in this build: ran it, which starts with an empty folder, since a
        // report left from an earlier run would vouch for a test the next run never ran; or found it up to
        // date, or took it from its cache, either of which vouches that the folder is the run of these very
        // inputs. Gradle asks a task's `onlyIf` on each of those ways and on none other, so that is where
        // the task says it was reached; the verifier, asked for without it — on its own, or with the task
        // left out of the build — finds no such word and refuses whatever an earlier run left in the folder.
        val path = task.path
        val reached = wiring.reached
        task.usesService(reached)
        task.onlyIf("the written-against-run count hears that this task was reached") {
            reached.get().reach(path)
            true
        }
        task.doFirst { reportsDir.get().asFile.deleteRecursively() }
        // The test task's own classes and classpath, read when the verifier's inputs are, since AGP
        // settles a unit test task's classpath after this runs: the classes Gradle hands TestNG from, and
        // the classpath the run stood on, which the verifier reads those classes through. Neither is
        // written into the run's own output, which Gradle's cache may hand to another checkout, where
        // paths of this one would name files that are not there.
        wiring.testClasses.from(Callable { task.testClassesDirs })
        wiring.runClasspath.from(Callable { task.classpath })
        // The folder's files, not the folder itself: the verifier counts the report inside it.
        wiring.reports.from(reportsDir.map { dir -> dir.asFileTree })
        task.finalizedBy(project.tasks.named(VERIFIER))
    }
}

/**
 * The test tasks Gradle reached in this build — ran, found up to date, or took from its cache — by path,
 * for the verifier to ask after: one build's answer, gone with the build.
 */
abstract class ReachedTestTasks : BuildService<BuildServiceParameters.None> {
    private val reached = ConcurrentHashMap.newKeySet<String>()

    fun reach(path: String) {
        reached += path
    }

    fun wasReached(path: String): Boolean = path in reached
}

/**
 * The check itself: read the compiled test classes, read TestNG's report of what it ran, and close the
 * two against each other — the report's classes matched to the compiled ones through the type hierarchy
 * the tests themselves define, so an inherited test answers for the class that declares it and a test
 * in an abstract class no run reaches is named as never run.
 */
abstract class VerifyWrittenAgainstRun : DefaultTask() {
    @get:Input
    abstract val moduleName: Property<String>

    /** The test task whose run this counts; none when no test task was wired to the count. */
    @get:Input
    @get:Optional
    abstract val testTask: Property<String>

    /** The test task's compiled test classes, where Gradle finds the classes it hands TestNG. */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val testClasses: ConfigurableFileCollection

    /** The classpath the test run stood on: reading the test classes resolves their types from it alone. */
    @get:Classpath
    abstract val runClasspath: ConfigurableFileCollection

    /** TestNG's report folder of the run: its `testng-results.xml`. */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val testngReports: ConfigurableFileCollection

    /** Which test tasks Gradle reached in this build. */
    @get:Internal
    abstract val reached: Property<ReachedTestTasks>

    @TaskAction
    fun compareWrittenAgainstRun() {
        val module = moduleName.get()
        val run =
            testTask.orNull
                ?: throw GradleException(
                    "$module: no test task is wired to this count, so it has no run to count; $VERIFIER runs as the " +
                        "finalizer of the project's test task.",
                )
        if (!reached.get().wasReached(run)) {
            throw GradleException(
                "$module: $VERIFIER counts the tests $run ran, as its finalizer, and $run was not part of this build, " +
                    "so its report folder holds whatever an earlier run left there: run $run, which runs the count after it.",
            )
        }
        val classDirs = testClasses.files.filter(File::isDirectory)
        val reports = testngReports.files.filter { it.name == TESTNG_REPORT }
        val counts =
            URLClassLoader(
                (classDirs + runClasspath.files).map { it.asUrl() }.toTypedArray(),
                ClassLoader.getPlatformClassLoader(),
            ).use { loader ->
                val reading = readWrittenTests(classDirs, loader)
                compareWrittenAgainstRun(
                    reading.tests,
                    readTestngReports(reports),
                    reading.unreadable,
                    reported = reports.isNotEmpty(),
                ) { child, parent -> isSubtypeOf(loader, child, parent) }
            }
        println("$module: ${counts.written} tests written, ${counts.ran} run by TestNG")
        if (!counts.balanced) {
            throw GradleException(
                buildString {
                    appendLine(
                        "$module compiles ${counts.written} tests and TestNG ran ${counts.ran} of them; every test " +
                            "the build compiles must be among the tests TestNG reports it ran, and nothing TestNG ran may " +
                            "stand outside the tests the build compiles:",
                    )
                    counts.problems.forEach(::appendLine)
                },
            )
        }
    }
}

/** A folder or jar on a classloader's list of places to read classes from. */
private fun File.asUrl(): URL = toURI().toURL()

/** Whether [child] is [parent] or stands under it, among the classes the test run could load, as TestNG saw them. */
private fun isSubtypeOf(
    loader: ClassLoader,
    child: String,
    parent: String,
): Boolean =
    try {
        loader.loadClass(parent).isAssignableFrom(loader.loadClass(child))
    } catch (failure: Throwable) {
        // A class the run's classpath cannot resolve matches no written test, so it is reported as a
        // run test outside the written ones — louder than letting an unreadable name slide past.
        false
    }
