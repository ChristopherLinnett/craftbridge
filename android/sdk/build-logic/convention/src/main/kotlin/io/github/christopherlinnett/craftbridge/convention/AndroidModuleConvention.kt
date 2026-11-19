package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.CommonExtension
import io.github.christopherlinnett.craftbridge.testcount.TestCountVerification
import org.gradle.api.GradleException
import org.gradle.api.JavaVersion
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.api.artifacts.ModuleDependency
import org.gradle.api.tasks.testing.Test
import org.gradle.kotlin.dsl.configure
import org.gradle.kotlin.dsl.getByType
import org.gradle.kotlin.dsl.withType
import org.jetbrains.kotlin.gradle.dsl.ExplicitApiMode
import org.jetbrains.kotlin.gradle.dsl.KotlinAndroidProjectExtension
import org.jetbrains.kotlin.gradle.tasks.AbstractKotlinCompile
import org.jetbrains.kotlin.gradle.tasks.KotlinCompilationTask
import java.io.File
import java.nio.file.Path
import java.nio.file.Paths

/**
 * The Lint checks no module runs: checks that only report that a newer version of something
 * outside the change exists. `AndroidGradlePluginVersion` reports a newer AGP or Gradle, and
 * `GradleDependency` a newer release of a library the build names; each of them fails the gate on
 * the day its subject ships, not on the day the module changed. The versions move deliberately:
 * the upstream update loop names each one with a newer release (plan §24), and it arrives in a change
 * of its own through `gradle/libs.versions.toml` or the wrapper, where a review sees it (§5 keeps every
 * version there once), not from Lint noticing a release.
 */
private val dayDependentLintChecks = setOf("AndroidGradlePluginVersion", "GradleDependency")

/**
 * The compiler arguments AGP adds to a compilation itself: `-Xuse-inline-scopes-numbers` on the
 * instrumented-test compilation (measured with AGP 9.5.1). None changes which warnings are reported. Any
 * other free argument could (`-nowarn`, `-Xsuppress-warning`, a warning level), so it is refused.
 */
private val agpCompilerArgs = setOf("-Xuse-inline-scopes-numbers")

/**
 * The generated bindings are exempt from explicit API mode, and from nothing else: each of their files
 * suppresses the two diagnostics that mode reports (`cargo xtask generate` writes that line, see
 * `rust/xtask/src/kotlin_bindings.rs`), since UniFFI declares its helpers with no visibility. The compiler
 * warns about every suppressed error, which warnings as errors would turn into a failed build; no compiler
 * option reaches less than the whole compilation, and no annotation silences that warning, so it is
 * silenced here for every file. The refusal it made moves to detekt, whose `ErrorSuppression` reads every
 * file but the bindings' folder ([GENERATED_BINDINGS]) and fails on any suppression of a compiler error:
 * written code, its tests and the command facade beside the bindings keep the mode whole.
 */
private const val QUIET_ERROR_SUPPRESSION = "-Xdont-warn-on-error-suppression"

/**
 * The conformance data the repository keeps for this module's Craft app, `conformance/<module>`, which
 * every platform's runner reads (AGENTS §1), or none when the repository holds none for the module. The
 * module's instrumented-test APK packs it as its assets from where it is, so the one corpus is never
 * copied or generated into the repository, and the library itself never ships it.
 */
private fun Project.conformanceFolder(): File? = repositoryRoot.resolve("conformance/$name").canonicalFile.takeIf { it.isDirectory }

/** The two kinds of Android module this build has, and the convention plugin each applies, which a module's
 * rules are read back in the name of. */
enum class AndroidModuleKind(
    val convention: String,
    /** A library's public surface is an API contract (explicit API mode); an app has none to keep. */
    val library: Boolean,
    /** The compiler arguments this kind's own plugins add, which change no warning: the Compose compiler's, for an app. */
    val compilerArgs: Set<String> = emptySet(),
) {
    LIBRARY("craftbridge.android.library", library = true),
    APPLICATION("craftbridge.android.application", library = false, compilerArgs = setOf("-Xallow-unstable-dependencies")),
}

/**
 * A convention plugin for one kind of Android module: the Android plugin of that kind, the setup every kind shares
 * ([configureAndroidModule]), and what only this kind needs ([configureKind]).
 */
abstract class AndroidModuleConventionPlugin(
    private val kind: AndroidModuleKind,
    private val androidPlugin: String,
) : Plugin<Project> {
    final override fun apply(target: Project) {
        target.pluginManager.apply(androidPlugin)
        target.configureAndroidModule(kind)
        target.configureKind()
    }

    /** What this kind of module has that the others have not, once the shared setup is in place. */
    protected abstract fun Project.configureKind()
}

/**
 * The Android setup every CraftBridge module shares, library or app: SDK levels, Java and Kotlin rules,
 * Lint strictness, where generated Kotlin is compiled from and what the unit tests run on. A module's
 * build file applies its convention plugin and names only its namespace and dependencies; the rules the
 * gate relies on are read back once that file has run, so it cannot set them back either.
 */
internal fun Project.configureAndroidModule(kind: AndroidModuleKind) {
    pluginManager.apply(QualityConventionPlugin::class.java)
    // AGP's tasks keep their outputs under folders they share, which Gradle's cleanup after a
    // skipped task prunes, so the module's tasks take turns at its build folder: otherwise a
    // module losing its last sources fails the build on some runs and not others.
    takeTurnsAtBuildFolder()
    // JNA ships its native dispatch library in an AAR beside the jar coordinate its POM names,
    // and an app that resolves the bare coordinate gets the jar and no native library. Every
    // dependency on it therefore asks for the AAR as an artifact — the selection an app's own
    // `@aar` notation would make — so that the module resolves it right and the metadata it
    // publishes tells its consumers the same: Gradle writes the POM's `type` and the module
    // metadata's artifact selector from this one selection. A module build file may hold only a
    // catalog alias (AGENTS §10), so the selection is made once here.
    val handlers = dependencies
    val jna = jnaModule()
    configurations.configureEach {
        withDependencies {
            withType(ModuleDependency::class.java).configureEach {
                if (group == jna.group && name == jna.name && artifacts.isEmpty()) {
                    artifacts.add(handlers.aarSelectorOf(jna))
                }
            }
        }
    }

    extensions.configure<CommonExtension> {
        compileSdk = catalogVersion("android-compileSdk").toInt()
        buildToolsVersion = catalogVersion("android-buildTools")
        defaultConfig.minSdk = catalogVersion("android-minSdk").toInt()
        val bytecode = JavaVersion.toVersion(catalogVersion("java-bytecode"))
        compileOptions.sourceCompatibility = bytecode
        compileOptions.targetCompatibility = bytecode
        // A warning left in place becomes a permanent one, so it fails the build here.
        lint.warningsAsErrors = true
        lint.abortOnError = true
        lint.disable += dayDependentLintChecks
        // Generated Kotlin — the command facade and the bindings — is compiled from a folder of
        // its own, which only `cargo xtask generate` writes (AGENTS §6), beside the written code.
        sourceSets
            .getByName("main")
            .kotlin.directories
            .add(GENERATED_KOTLIN)
        // The one corpus is the instrumented checks' assets and nothing else's: the check on source
        // folders below lets this one folder be outside the module, as the assets of this source
        // set alone.
        conformanceFolder()?.let { corpus ->
            sourceSets
                .getByName("androidTest")
                .assets.directories
                .add(corpus.path)
        }
    }

    // Every module's unit tests run on TestNG, through kotlin-test's binding to it: kotlin-test
    // binds to JUnit 4 by default, whose EPL-1.0 is on no branch of the allowlist (AGENTS §8), and
    // Gradle's default runner discovers no test the TestNG binding marks, so the framework and
    // its binding are chosen here together, once for every module. The binding drags a TestNG of
    // its own along transitively, so the TestNG is named beside it, as the catalog names it: strict,
    // so the tests run on that version or the build fails, never on one nobody read.
    dependencies.addProvider("testImplementation", catalogLibrary("kotlin-test-testng"))
    dependencies.addProvider("testImplementation", catalogLibrary("testng"))
    // Each unit test task then runs on TestNG, named there with TestNG's own reporter, and answers
    // for what its run was: the tests it compiles are compared with the tests TestNG reports it ran,
    // by the one definition in build-logic's build `shared`, so a test the run silently skips fails
    // the module by name.
    TestCountVerification.install(this)
    tasks.withType<Test>().configureEach { TestCountVerification.verify(this) }

    // AGP 9 compiles Kotlin itself and registers this extension for it.
    extensions.configure<KotlinAndroidProjectExtension> {
        // The public surface is an API contract: every declaration states its visibility. An app has
        // none to keep.
        if (kind.library) explicitApi()
        compilerOptions {
            allWarningsAsErrors.set(true)
            freeCompilerArgs.add(QUIET_ERROR_SUPPRESSION)
        }
    }

    // detekt is the gate's Kotlin linter (plan D-28): the never-crash calls of AGENTS §3, the JNI
    // confinement of §4, the package rule of §9 and the refusal of suppressions of §10 are its
    // rules, read from Kotlin's own syntax tree rather than from text. As with Spotless and Lint
    // the setup lives here once and no module build file names it.
    configureDetekt()

    // Every Kotlin and Java file a module keeps under `src` must be compiled by something: Spotless
    // formats the written Kotlin there, so a file no compilation reads would be formatted but never
    // type-checked. The compilations are the ones detekt reads, off AGP's variant API — one walk
    // names them for both.
    refuseUnreadSources()

    // A module's build file runs after this plugin and could set any rule above back, as a crate
    // could weaken its `[lints]` if the rule checks did not read them. So the rules are read again
    // once the build file has run: the extensions after evaluation, when Android locks its DSL, so
    // Lint's settings are final there; the source folders at the same moment, because one pointing
    // outside the module would compile code no dependency names; and each Kotlin compilation just
    // before it runs, since a compile task's own options outrank the extension's and the Kotlin
    // extension stays open to a later `afterEvaluate`.
    afterEvaluate {
        val android = extensions.getByType<CommonExtension>()
        val lint = android.lint
        val kotlin = extensions.getByType<KotlinAndroidProjectExtension>()
        val changed =
            buildList {
                if (!lint.warningsAsErrors || !lint.abortOnError || lint.ignoreWarnings) add("Lint's warnings as errors")
                if (lint.disable != dayDependentLintChecks || lint.checkOnly.isNotEmpty() || lint.informational.isNotEmpty()) {
                    add("the Lint checks that run")
                }
                if (lint.lintConfig != null || lint.baseline != null) add("Lint's configuration file or baseline")
                if (kind.library && kotlin.explicitApi != ExplicitApiMode.Strict) add("explicit API mode")
            }
        if (changed.isNotEmpty()) {
            throw GradleException(
                "$path changes ${changed.joinToString()}, which ${kind.convention} sets for every module",
            )
        }
        // AGENTS §2 keeps one module's code in its own folder: a source directory pointing
        // elsewhere would compile another module's — or any outside — code into this one, or
        // pack its resources, assets or native libraries into the AAR, with no dependency edge,
        // no entry in the project graph and no licence trail. Android's source sets are read
        // back, as the folder names it stores for every kind of source it packs: AGP 9 compiles
        // Kotlin itself from them, and Kotlin's own source-set DSL is deprecated in their favour.
        val corpus = conformanceFolder()?.toPath()

        // An absolute folder name is a path of its own; a relative one belongs to the module.
        // `File(projectDir, name)` would fold a leading `/` into a relative path and hide the
        // first case.
        fun folderOf(declared: String): Path =
            Paths
                .get(declared)
                .let { path -> if (path.isAbsolute) path else projectDir.toPath().resolve(path) }
                .toFile()
                .canonicalFile
                .toPath()
        val roots =
            android.sourceSets.flatMap { set ->
                listOf(
                    set.java,
                    set.kotlin,
                    set.resources,
                    set.res,
                    set.assets,
                    set.aidl,
                    set.renderscript,
                    set.baselineProfiles,
                    set.keepRules,
                    set.jniLibs,
                    set.shaders,
                    set.mlModels,
                ).flatMap { kind ->
                    // The corpus is the one folder outside the module that a source set may name, and
                    // only the instrumented checks' assets: named by `main`, or as any other kind of
                    // source, it would be packed into what ships.
                    val packedInTests = set.name == "androidTest" && kind === set.assets
                    kind.directories.map(::folderOf).filterNot { packedInTests && it == corpus }
                }
            }
        val outside = roots.distinct().filterNot { it.startsWith(projectDir.canonicalFile.toPath()) }
        if (outside.isNotEmpty()) {
            throw GradleException(
                "$path builds from ${outside.joinToString()}, outside its own folder: a module's code, resources and native libraries live in the module",
            )
        }
    }
    val module = path
    tasks.withType<KotlinCompilationTask<*>>().configureEach {
        // Kotlin leaves test compilations out of explicit API mode by its own rule; a main one
        // never is.
        val shipped = !name.contains("Test")
        doFirst("checkKotlinRules") {
            val task = this as KotlinCompilationTask<*>
            val options = task.compilerOptions
            if (!options.allWarningsAsErrors.get() || options.suppressWarnings.get() ||
                (options.freeCompilerArgs.get() - agpCompilerArgs - kind.compilerArgs - QUIET_ERROR_SUPPRESSION).isNotEmpty()
            ) {
                throw GradleException(
                    "$module changes Kotlin's warnings as errors, which ${kind.convention} sets for every module",
                )
            }
            if (kind.library && shipped && (task as? AbstractKotlinCompile<*>)?.explicitApiMode?.orNull != ExplicitApiMode.Strict) {
                throw GradleException("$module changes explicit API mode, which ${kind.convention} sets for every module")
            }
        }
    }
}
