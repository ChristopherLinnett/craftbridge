package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.CommonExtension
import com.android.build.api.variant.Component
import org.gradle.api.DefaultTask
import org.gradle.api.Project
import org.gradle.api.Task
import org.gradle.api.attributes.Usage
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.FileCollection
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.provider.Provider
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
 * The plumbing one detekt run of its own needs: the tool, the build's own rules, the files to read, the
 * configs to merge, the report to write, and the JVM of its own to run in. Two task bodies put that one
 * definition to opposite uses — [CraftbridgeDetekt], where any finding fails the build, and
 * [CraftbridgeGuideKotlin], where every finding is the answer being asked for — and neither says it twice.
 */
abstract class CraftbridgeDetektRun : DefaultTask() {
    /** The detekt command-line tool and its own libraries, at the catalog's version. */
    @get:Classpath
    abstract val toolClasspath: ConfigurableFileCollection

    /** The rules this build adds to detekt's own, loaded as the tool's plugin jar. */
    @get:Classpath
    abstract val rulePlugins: ConfigurableFileCollection

    /** The folders and files this module's components compile, as AGP's variant API lists them. */
    @get:InputFiles
    @get:SkipWhenEmpty
    @get:IgnoreEmptyDirectories
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val checkedSources: ConfigurableFileCollection

    /** The profiles the run merges into its one config. */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val configFiles: ConfigurableFileCollection

    /** The run's report over the checked files, written by the tool whether or not it found anything. */
    @get:OutputFile
    abstract val reportFile: RegularFileProperty

    @get:Inject
    protected abstract val execOperations: ExecOperations

    /** A collection spelt the way detekt reads several inputs: absolute paths, separated the host's way. */
    protected fun ConfigurableFileCollection.joined(): String = files.joinToString(File.pathSeparator) { it.absolutePath }
}

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
abstract class CraftbridgeDetekt : CraftbridgeDetektRun() {
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

    @TaskAction
    fun analyse() {
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

/** The source folder of every module's generated Kotlin, which it compiles beside its written code. */
internal const val GENERATED_KOTLIN = "src/generated/kotlin"

/**
 * Where the generated bindings sit inside [GENERATED_KOTLIN]: the folders of §9's `ffi` packages. The
 * bindings are the tool's own code, held to §3 alone; every other generated file, the command facade
 * above all, is written code written for you. Spotless and detekt both split the folder at this line,
 * so the facade is formatted and checked as written code and the bindings are neither.
 */
internal const val GENERATED_BINDINGS = "io/github/christopherlinnett/craftbridge/ffi"

/**
 * detekt for every component AGP builds for the module, as AGP's variant API lists them: `detektMain`
 * checks each variant's written code and the generated facade beside it with the written and shipped
 * profiles, its generated bindings with the base profile alone, and `detektTest` each variant's test
 * components with the written one. A build type or product flavor added later brings its own tasks with
 * it; nothing here names one.
 */
internal fun Project.configureDetekt() {
    val android = extensions.getByType<CommonExtension>()
    val androidComponents = androidComponents()
    // Tool classpaths of the build's own, detached from the module's configurations: no module code
    // compiles or ships against them, so the dependency direction the settings check reads off every
    // module configuration (§2) is not theirs to answer, and a module build file cannot reach them.
    val detektCli = configurations.detachedConfiguration(dependencies.create(catalogLibrary("detekt-cli").get()))
    // settings.gradle.kts substitutes the included build's project for this coordinate.
    val detektRules =
        configurations.detachedConfiguration(dependencies.create("io.github.christopherlinnett.craftbridge.build:detekt-rules")).apply {
            attributes.attribute(Usage.USAGE_ATTRIBUTE, objects.named(Usage::class.java, Usage.JAVA_RUNTIME))
        }

    val profiles = buildLogicFolder.resolve("detekt-rules/profiles")
    val base = profiles.resolve("base.yml")
    val written = profiles.resolve("written.yml")
    val shipped = profiles.resolve("shipped.yml")
    val guide = profiles.resolve("guide.yml")
    val packages =
        tasks.register<CraftbridgeDetektConfig>("craftbridgeDetektConfig") {
            description = "Writes the packages this module's sources and bindings declare, and its folder, for the rules"
            group = "verification"
            // Read when the task runs, after the module's own build file has named its namespace:
            // that namespace for written code, and §9's ffi spelling of it for the bindings.
            val namespace =
                provider {
                    android.namespace ?: error("$path names no namespace: the convention plugin derives the package rule from it")
                }
            writtenPackage.set(namespace)
            bindingsPackage.set(namespace.map { "${it.substringBeforeLast('.')}.ffi.${it.substringAfterLast('.')}" })
            moduleDirectory.set(projectDir.canonicalFile.invariantSeparatorsPath)
            generatedFolder.set(projectDir.resolve(GENERATED_KOTLIN).canonicalFile.invariantSeparatorsPath)
            writtenConfig.set(layout.buildDirectory.file("craftbridge-detekt/written-packages.yml"))
            bindingsConfig.set(layout.buildDirectory.file("craftbridge-detekt/bindings-packages.yml"))
        }
    val mainChecks = aggregate("detektMain", "every variant's shipped code, generated facade and generated bindings")
    val testChecks = aggregate("detektTest", "every variant's test components")
    val generated = projectDir.resolve(GENERATED_KOTLIN)
    // AGP lists the generated folder as one root, so each run takes its half of it as a file tree of
    // its own, which Gradle walks when the run executes: the facade with the written code, the
    // bindings ([GENERATED_BINDINGS]) alone. Neither half can move into the other's profile without
    // moving its package, which each run's package rule checks.
    val facade =
        fileTree(generated) {
            include("**/*.kt")
            exclude("$GENERATED_BINDINGS/**")
        }
    val bindings = fileTree(generated) { include("$GENERATED_BINDINGS/**/*.kt") }
    val allGenerated = fileTree(generated) { include("**/*.kt") }
    val nothing = files()

    fun register(
        name: String,
        component: Component,
        what: String,
        directories: (File) -> Boolean,
        generatedFiles: FileCollection,
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
            // AGP lists a source set's folders whether or not the module has them, and the analyser
            // reads only folders that exist; a declared-but-unused folder is no finding.
            checkedSources.from(component.kotlinFolders(path).map { folders -> folders.filter(directories) })
            checkedSources.from(generatedFiles)
            configFiles.from(profileFiles, packages.flatMap(packagesFile))
            reportFile.set(layout.buildDirectory.file("craftbridge-detekt/$name.md"))
        }

    forEachComponent(
        main = { variant ->
            val name = "detekt" + variant.name.replaceFirstChar(Char::uppercaseChar)
            // Written code, and the generated facade with it, is what the never-crash checks were
            // written for; only the bindings are held to §3 alone, as Spotless leaves them unformatted.
            val code =
                register(
                    name,
                    variant,
                    "written code and the generated facade",
                    { it != generated },
                    facade,
                    listOf(base, written, shipped),
                    CraftbridgeDetektConfig::writtenConfig,
                )
            val bindings =
                register(
                    "${name}Bindings",
                    variant,
                    "generated bindings",
                    // Nothing from AGP's own folders: the bindings' half of the generated folder is its
                    // file tree, and every written folder stays out of their base profile.
                    { false },
                    bindings,
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
                    nothing,
                    listOf(base, written),
                    CraftbridgeDetektConfig::writtenConfig,
                )
            testChecks.configure { dependsOn(test) }
        },
    )

    // The guide check's Kotlin answer for this module: the folders the very same variant API lists for
    // every component — written, generated, test, whatever a compilation reads — are the files it
    // says the build compiles, so no second list of folders exists (§3, §10), and the lines those
    // files compile to nothing come from `CompilesToNothing` in a light run over them alone.
    val guideSources = mutableListOf<Any>()
    forEachComponent(
        main = { variant -> guideSources.add(variant.kotlinFolders(path)) },
        test = { nested -> guideSources.add(nested.kotlinFolders(path)) },
    )
    tasks.register<CraftbridgeGuideKotlin>("guideKotlin") {
        description = "Writes which Kotlin this module compiles, and which lines of it compile to nothing, for the guide check"
        group = "verification"
        toolClasspath.from(detektCli)
        rulePlugins.from(detektRules)
        checkedSources.from(guideSources)
        checkedSources.from(allGenerated)
        // guide.yml alone: whether a line compiles to nothing, the parser alone says, so this run
        // resolves no types and carries no classpath, and its config names only the rule that answers.
        configFiles.from(guide)
        reportFile.set(layout.buildDirectory.file("craftbridge-detekt/guideKotlin.xml"))
        // The answer is written into the folder xtask names when it asks, as a file the guide check
        // reads once the task has run and removes after: an up-to-date task would write no answer at
        // all and the guide check would find a module compiling nothing. What the task writes follows
        // the files as they are now, and it must answer every time it is asked.
        outputs.upToDateWhen { false }
    }
}

/**
 * The folders AGP lists for the component's Kotlin, the ones that exist: AGP lists a source set's
 * folders whether or not the module has them, and no tool reads a folder that is not there. A
 * component listing no Kotlin at all has nothing for detekt to read, which is not a thing a module
 * quietly gets to be.
 */
private fun Component.kotlinFolders(where: String): Provider<List<File>> {
    val kotlin = sources.kotlin ?: error("$where: $name has no Kotlin sources for detekt to read")
    return kotlin.all.map { folders -> folders.map { it.asFile }.filter { it.isDirectory } }
}

private fun Project.aggregate(
    name: String,
    what: String,
): TaskProvider<Task> =
    tasks.register(name) {
        description = "The gate's Kotlin rules over $what"
        group = "verification"
    }
