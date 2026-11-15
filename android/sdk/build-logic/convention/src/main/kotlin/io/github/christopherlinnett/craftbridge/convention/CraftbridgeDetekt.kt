package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.LibraryExtension
import com.android.build.api.variant.Component
import com.android.build.api.variant.LibraryAndroidComponentsExtension
import org.gradle.api.DefaultTask
import org.gradle.api.Project
import org.gradle.api.Task
import org.gradle.api.attributes.Usage
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Classpath
import org.gradle.api.tasks.IgnoreEmptyDirectories
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.SkipWhenEmpty
import org.gradle.api.tasks.TaskAction
import org.gradle.api.tasks.TaskProvider
import org.gradle.kotlin.dsl.getByType
import org.gradle.kotlin.dsl.register
import org.gradle.process.ExecOperations
import java.io.File
import javax.inject.Inject

/**
 * One detekt run over one set of a module's sources, launched from the detekt command line in a JVM of
 * its own. detekt's analysis starts the Kotlin compiler's IntelliJ core, which sets IntelliJ's
 * JVM-wide settings as it starts (`idea.config.path` among them) — the settings Lint reads inside
 * Gradle's JVM — so it never runs in that JVM. detekt's own Gradle plugin is not applied either: every
 * released version of it calls `Configuration.setVisible`, which the pinned Gradle deprecates, so it
 * would fail every `--warning-mode=fail` build that runs it, and it runs the analyser inside Gradle's
 * JVM by default. The command line is the same analyser the plugin runs.
 *
 * Every run resolves types (`--analysis-mode full`, with the component's compile and boot classpaths):
 * detekt leaves a rule that needs resolution silently off in a run without it, so no run may lack it. A
 * component with nothing to check is NO-SOURCE — it says so rather than pretending to have checked
 * something — and any finding fails the task, because the profiles it is given let no issue stand.
 */
abstract class CraftbridgeDetekt : DefaultTask() {
    /** The detekt command-line tool and its own libraries, at the catalog's version. */
    @get:Classpath
    abstract val toolClasspath: ConfigurableFileCollection

    /** The rules this build adds to detekt's own, loaded as the tool's plugin jar. */
    @get:Classpath
    abstract val rulePlugins: ConfigurableFileCollection

    /**
     * What the checked sources compile against, the Kotlin stdlib among it (the run passes `-no-stdlib`,
     * so detekt brings no copy of its own): a rule that resolves types sees what the compiler sees.
     */
    @get:Classpath
    abstract val compileClasspath: ConfigurableFileCollection

    /** The platform's own classes, which the compile classpath leaves to the boot classpath, as the compiler does. */
    @get:Classpath
    abstract val bootClasspath: ConfigurableFileCollection

    /** The bytecode level the sources compile to, which decides what the analysis may inline from the classpath. */
    @get:Input
    abstract val jvmTarget: Property<String>

    @get:InputFiles
    @get:SkipWhenEmpty
    @get:IgnoreEmptyDirectories
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val checkedSources: ConfigurableFileCollection

    /** The profiles from build-logic, then the module's packages, which detekt merges into one config. */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val configFiles: ConfigurableFileCollection

    /** The run's findings as Markdown, written by the tool whether or not there were any. */
    @get:OutputFile
    abstract val reportFile: RegularFileProperty

    @get:Inject
    protected abstract val execOperations: ExecOperations

    @TaskAction
    fun analyse() {
        fun ConfigurableFileCollection.joined() = files.joinToString(File.pathSeparator) { it.absolutePath }
        // Full mode resolves types against both halves of the classpath, and detekt leaves a call it
        // cannot resolve unreported. Without the compile classpath the stdlib's `exitProcess` and `TODO`
        // pass in silence: the run passes `-no-stdlib`, so the stdlib arrives on that half alone.
        // Without the boot classpath every call through a platform type passes in silence: `android.jar`
        // sits on that half alone, and a call it cannot resolve is the one kind detekt keeps quiet
        // about — the running JDK still supplies plain `java.lang`, so only platform calls show it.
        // Each half is required on its own, since either alone still makes a classpath that looks present.
        for ((half, files) in listOf("compile classpath" to compileClasspath, "boot classpath" to bootClasspath)) {
            if (files.isEmpty) {
                error("$path has no $half: full mode resolves the checked calls against it, and a call it cannot resolve passes unreported")
            }
        }
        execOperations.javaexec {
            classpath = toolClasspath
            mainClass.set("dev.detekt.cli.Main")
            // The tool prints each finding and exits non-zero on any; Gradle fails the task on a
            // non-zero exit, so a finding is the task's own failure, console and all. The classpath
            // carries the stdlib the sources compile against, so detekt adds no copy of its own.
            args(
                // Every finding fails the run whatever its severity: by default detekt fails only on an
                // error, and a profile that set a rule's or a rule set's `severity` lower would leave its
                // findings printed and passing.
                "--fail-on-severity",
                "Info",
                "--input",
                checkedSources.joined(),
                "--config",
                configFiles.joined(),
                "--plugins",
                rulePlugins.joined(),
                "--analysis-mode",
                "full",
                "--classpath",
                (compileClasspath + bootClasspath).joinToString(File.pathSeparator) { it.absolutePath },
                "--jvm-target",
                jvmTarget.get(),
                "--report",
                // The report's id is `markdown`, not the `md` the tool's own help prints: no provider
                // answers `md`, the CLI fails to say so, and the file the task declares as its output
                // would never be written — the analysis would go into no file at all.
                "markdown:" + reportFile.get().asFile.absolutePath,
                "-no-stdlib",
                "-no-reflect",
            )
        }
    }
}

/** The generated bindings' source folder, which every module compiles beside its written code. */
internal const val GENERATED_KOTLIN = "src/generated/kotlin"

/**
 * detekt for every component AGP builds for the module, as AGP's variant API lists them: `detektMain`
 * checks each variant's written code with the written and shipped profiles and its generated bindings
 * with the base profile alone, and `detektTest` each variant's test components with the written one.
 * A build type or product flavor added later brings its own tasks with it; nothing here names one.
 */
internal fun Project.configureDetekt() {
    val android = extensions.getByType<LibraryExtension>()
    val androidComponents = extensions.getByType<LibraryAndroidComponentsExtension>()
    // Tool classpaths of the build's own, detached from the module's configurations: no module code
    // compiles or ships against them, so the dependency direction the settings check reads off every
    // module configuration (§2) is not theirs to answer, and a module build file cannot reach them.
    val detektCli = configurations.detachedConfiguration(dependencies.create(catalogLibrary("detekt-cli").get()))
    // settings.gradle.kts substitutes the included build's project for this coordinate.
    val detektRules =
        configurations.detachedConfiguration(dependencies.create("io.github.christopherlinnett.craftbridge.build:detekt-rules")).apply {
            attributes.attribute(Usage.USAGE_ATTRIBUTE, objects.named(Usage::class.java, Usage.JAVA_RUNTIME))
        }

    val profiles = rootDir.resolve("build-logic/detekt-rules/profiles")
    val base = profiles.resolve("base.yml")
    val written = profiles.resolve("written.yml")
    val shipped = profiles.resolve("shipped.yml")
    val packages =
        tasks.register<CraftbridgeDetektConfig>("craftbridgeDetektConfig") {
            description = "Writes the packages this module's sources and bindings declare, and its folder, for the rules"
            group = "verification"
            // Read when the task runs, after the module's own build file has named its namespace:
            // that namespace for written code, and §9's ffi spelling of it for the bindings.
            val namespace =
                provider {
                    android.namespace ?: error("$path names no namespace: craftbridge.android.library derives the package rule from it")
                }
            writtenPackage.set(namespace)
            bindingsPackage.set(namespace.map { "${it.substringBeforeLast('.')}.ffi.${it.substringAfterLast('.')}" })
            moduleDirectory.set(projectDir.canonicalFile.invariantSeparatorsPath)
            writtenConfig.set(layout.buildDirectory.file("craftbridge-detekt/written-packages.yml"))
            bindingsConfig.set(layout.buildDirectory.file("craftbridge-detekt/bindings-packages.yml"))
        }
    val mainChecks = aggregate("detektMain", "every variant's shipped code and generated bindings")
    val testChecks = aggregate("detektTest", "every variant's test components")
    val generated = projectDir.resolve(GENERATED_KOTLIN)

    fun register(
        name: String,
        component: Component,
        what: String,
        directories: (File) -> Boolean,
        profileFiles: List<File>,
        packagesFile: (CraftbridgeDetektConfig) -> RegularFileProperty,
    ): TaskProvider<CraftbridgeDetekt> =
        tasks.register<CraftbridgeDetekt>(name) {
            description = "The gate's Kotlin rules over ${component.name}'s $what"
            group = "verification"
            toolClasspath.from(detektCli)
            rulePlugins.from(detektRules)
            compileClasspath.from(component.compileClasspath)
            bootClasspath.from(androidComponents.sdkComponents.bootClasspath)
            jvmTarget.set(catalogVersion("java-bytecode"))
            val kotlin = component.sources.kotlin ?: error("$path: ${component.name} has no Kotlin sources for detekt to read")
            // AGP lists a source set's folders whether or not the module has them, and the analyser
            // reads only folders that exist; a declared-but-unused folder is no finding.
            checkedSources.from(kotlin.all.map { all -> all.map { it.asFile }.filter { it.isDirectory && directories(it) } })
            configFiles.from(profileFiles, packages.flatMap(packagesFile))
            reportFile.set(layout.buildDirectory.file("craftbridge-detekt/$name.md"))
        }

    forEachComponent(
        main = { variant ->
            val name = "detekt" + variant.name.replaceFirstChar(Char::uppercaseChar)
            // Written code is what the never-crash checks were written for; the generated bindings change
            // through their generator instead (§6), as the Spotless exclusion has it.
            val code =
                register(
                    name,
                    variant,
                    "written code",
                    { it != generated },
                    listOf(base, written, shipped),
                    CraftbridgeDetektConfig::writtenConfig,
                )
            val bindings =
                register(
                    "${name}Bindings",
                    variant,
                    "generated bindings",
                    { it == generated },
                    listOf(base),
                    CraftbridgeDetektConfig::bindingsConfig,
                )
            mainChecks.configure { dependsOn(code, bindings) }
        },
        test = { nested ->
            val test =
                register(
                    "detekt" + nested.name.replaceFirstChar(Char::uppercaseChar),
                    nested,
                    "sources",
                    { true },
                    listOf(base, written),
                    CraftbridgeDetektConfig::writtenConfig,
                )
            testChecks.configure { dependsOn(test) }
        },
    )
}

private fun Project.aggregate(
    name: String,
    what: String,
): TaskProvider<Task> =
    tasks.register(name) {
        description = "The gate's Kotlin rules over $what"
        group = "verification"
    }
