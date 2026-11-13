package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.LibraryExtension
import org.gradle.api.GradleException
import org.gradle.api.JavaVersion
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure
import org.gradle.kotlin.dsl.getByType
import org.gradle.kotlin.dsl.withType
import org.jetbrains.kotlin.gradle.dsl.ExplicitApiMode
import org.jetbrains.kotlin.gradle.dsl.KotlinAndroidProjectExtension
import org.jetbrains.kotlin.gradle.tasks.AbstractKotlinCompile
import org.jetbrains.kotlin.gradle.tasks.KotlinCompilationTask

/**
 * The one Lint check no module runs. It reports that a newer AGP or Gradle exists, which depends on the
 * day, not on the change: it would fail the gate whenever either ships a release. The versions move
 * deliberately, in gradle/libs.versions.toml and the wrapper.
 */
private val dayDependentLintChecks = setOf("AndroidGradlePluginVersion")

/**
 * The compiler arguments AGP adds to a compilation itself: `-Xuse-inline-scopes-numbers` on the
 * instrumented-test compilation (measured with AGP 9.5.1). None changes which warnings are reported. Any
 * other free argument could (`-nowarn`, `-Xsuppress-warning`, a warning level), so it is refused.
 */
private val agpCompilerArgs = setOf("-Xuse-inline-scopes-numbers")

/**
 * The Android library setup every CraftBridge module shares: SDK levels, Java and Kotlin rules,
 * Lint strictness, the consumer R8 rules and where generated bindings are compiled from. A
 * module's build file applies this plugin and names only its namespace and dependencies; the rules the
 * gate relies on are read back once that file has run, so it cannot set them back either.
 */
class AndroidLibraryConventionPlugin : Plugin<Project> {
    override fun apply(target: Project): Unit =
        with(target) {
            pluginManager.apply("com.android.library")
            pluginManager.apply(QualityConventionPlugin::class.java)

            extensions.configure<LibraryExtension> {
                compileSdk = catalogVersion("android-compileSdk").toInt()
                buildToolsVersion = catalogVersion("android-buildTools")
                defaultConfig {
                    minSdk = catalogVersion("android-minSdk").toInt()
                    // Published inside the AAR, so an app that shrinks its code keeps what the native
                    // bindings need without knowing the details.
                    consumerProguardFiles("consumer-rules.pro")
                }
                compileOptions {
                    val bytecode = JavaVersion.toVersion(catalogVersion("java-bytecode"))
                    sourceCompatibility = bytecode
                    targetCompatibility = bytecode
                }
                lint {
                    // A warning left in place becomes a permanent one, so it fails the build here.
                    warningsAsErrors = true
                    abortOnError = true
                    disable += dayDependentLintChecks
                }
                // Generated bindings are compiled from their own folder, so no tool ever formats,
                // lints or hand-edits them alongside the written code.
                sourceSets
                    .getByName("main")
                    .kotlin.directories
                    .add("src/generated/kotlin")
            }

            // AGP 9 compiles Kotlin itself and registers this extension for it.
            extensions.configure<KotlinAndroidProjectExtension> {
                // The public surface is an API contract: every declaration states its visibility.
                explicitApi()
                compilerOptions {
                    allWarningsAsErrors.set(true)
                }
            }

            // A module's build file runs after this plugin and could set any rule above back, as a crate
            // could weaken its `[lints]` if the rule checks did not read them. So the rules are read again
            // once the build file has run: the extensions after evaluation, when Android locks its DSL, so
            // Lint's settings are final there; the source folders at the same moment, because one pointing
            // outside the module would compile code no dependency names; and each Kotlin compilation just
            // before it runs, since a compile task's own options outrank the extension's and the Kotlin
            // extension stays open to a later `afterEvaluate`.
            afterEvaluate {
                val android = extensions.getByType<LibraryExtension>()
                val lint = android.lint
                val kotlin = extensions.getByType<KotlinAndroidProjectExtension>()
                val changed =
                    buildList {
                        if (!lint.warningsAsErrors || !lint.abortOnError || lint.ignoreWarnings) add("Lint's warnings as errors")
                        if (lint.disable != dayDependentLintChecks || lint.checkOnly.isNotEmpty() || lint.informational.isNotEmpty()) {
                            add("the Lint checks that run")
                        }
                        if (lint.lintConfig != null || lint.baseline != null) add("Lint's configuration file or baseline")
                        if (kotlin.explicitApi != ExplicitApiMode.Strict) add("explicit API mode")
                    }
                if (changed.isNotEmpty()) {
                    throw GradleException(
                        "$path changes ${changed.joinToString()}, which craftbridge.android.library sets for every module",
                    )
                }
                // AGENTS §2 keeps one module's code in its own folder: a source directory pointing
                // elsewhere would compile another module's — or any outside — code into this one, or
                // pack its resources, assets or native libraries into the AAR, with no dependency edge,
                // no entry in the project graph and no licence trail. Android's source sets are read
                // back, as the folder names it stores for every kind of source it packs: AGP 9 compiles
                // Kotlin itself from them, and Kotlin's own source-set DSL is deprecated in their favour.
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
                        ).flatMap { kind -> kind.directories }
                    }
                val outside =
                    roots
                        // An absolute folder name is a path of its own; a relative one belongs to the
                        // module. `File(projectDir, name)` would fold a leading `/` into a relative
                        // path and hide the first case.
                        .map { root ->
                            java.nio.file.Paths
                                .get(root)
                                .let { declared ->
                                    if (declared.isAbsolute) declared else projectDir.toPath().resolve(declared)
                                }.toFile()
                                .canonicalFile
                                .toPath()
                        }.distinct()
                        .filterNot { it.startsWith(projectDir.canonicalFile.toPath()) }
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
                        (options.freeCompilerArgs.get() - agpCompilerArgs).isNotEmpty()
                    ) {
                        throw GradleException(
                            "$module changes Kotlin's warnings as errors, which craftbridge.android.library sets for every module",
                        )
                    }
                    if (shipped && (task as? AbstractKotlinCompile<*>)?.explicitApiMode?.orNull != ExplicitApiMode.Strict) {
                        throw GradleException("$module changes explicit API mode, which craftbridge.android.library sets for every module")
                    }
                }
            }
        }
}
