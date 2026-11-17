package io.github.christopherlinnett.craftbridge.convention

import com.diffplug.gradle.spotless.BaseKotlinExtension
import com.diffplug.gradle.spotless.SpotlessExtension
import com.diffplug.gradle.spotless.SpotlessTask
import io.github.christopherlinnett.craftbridge.licences.LicenceListing
import org.gradle.api.GradleException
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.kotlin.dsl.configure

/** The runtime classpaths whose modules the licence check reads: what ships, and what each kind of test runs with. */
private val licenceClasspaths = listOf("releaseRuntimeClasspath", "debugUnitTestRuntimeClasspath", "debugAndroidTestRuntimeClasspath")

/**
 * The formatting rules for every Kotlin file of the Android build: Spotless running ktlint. A module
 * checks its own written Kotlin and build script; the build's root project checks what no module owns,
 * its own scripts and the convention plugins in build-logic, an included build this build's tasks
 * cannot otherwise reach. The repository's .editorconfig is the one definition of the style; nothing
 * here repeats it. What each Spotless task will really compare is read back after the project's own
 * build file has run, so no project can quietly decide for itself what gets formatted. The root project
 * also lists what the library modules ship, for the licence check.
 */
class QualityConventionPlugin : Plugin<Project> {
    override fun apply(target: Project): Unit =
        with(target) {
            pluginManager.apply("com.diffplug.spotless")
            val ktlintVersion = catalogVersion("ktlint")
            // The repository's one .editorconfig, named so its rules reach ktlint as an input of every
            // Spotless task. ktlint still merges any other .editorconfig between a file and this one, so
            // the gate refuses every other copy (`cargo xtask rules`, machinery), which is what keeps the
            // style in this file alone. The build sits at android/sdk.
            val editorConfig = rootDir.resolve("../../.editorconfig")
            // A copy that sat there while ktlint ran is no input of the task, so Gradle would replay that
            // run's verdict, from this checkout or from the machine's build cache, after the copy is gone.
            // Every Spotless task therefore runs every time and stores nothing: ktlint reads the tree as it
            // is, whatever else it reads.
            tasks.withType(SpotlessTask::class.java).configureEach {
                outputs.upToDateWhen { false }
                outputs.cacheIf { false }
            }

            // Named apart from Spotless's own ktlint(), which a call inside its blocks would reach first.
            fun BaseKotlinExtension.repositoryKtlint() {
                ktlint(ktlintVersion).setEditorConfigPath(editorConfig)
            }

            // Every target is a tree rooted where the sources are, or a named file. A pattern, and an
            // exclusion pattern above all, makes Gradle walk the whole project folder, build/ included,
            // while the compile tasks running beside these ones rewrite it, and the walk then fails on a
            // file that vanished under it.
            val atRoot = this == rootProject
            val buildLogicOutputs = listOf("build/**", "*/build/**", ".gradle/**", "*/.gradle/**", ".kotlin/**", "*/.kotlin/**")

            // What this plugin formats, kept to itself so the read-back below can compare against it.
            // The build-logic trees exclude the included build's own output folders, which hold copies
            // of these scripts under `build`: each project's own, at the top of the included build and of
            // its one project, never a source folder that happens to be called `build` (the convention
            // build's group is `…craftbridge.build`, and a package of that name could sit in its sources).
            val writtenKotlin =
                if (atRoot) {
                    fileTree("build-logic") {
                        include("**/*.kt")
                        exclude(buildLogicOutputs)
                    }
                } else {
                    // Every source set. Generated Kotlin is formatted like written Kotlin, and fails
                    // the same way if it is not — the generator is held to the style, never the other
                    // way round. Only the bindings, which their tool lays out in its own shape, sit
                    // outside: detekt splits the generated folder at the same line (GENERATED_BINDINGS).
                    fileTree("src") {
                        include("**/*.kt")
                        exclude("${GENERATED_KOTLIN.removePrefix("src/")}/$GENERATED_BINDINGS/**")
                    }
                }
            val writtenGradle =
                if (atRoot) {
                    files("build.gradle.kts", "settings.gradle.kts") +
                        fileTree("build-logic") {
                            include("**/*.gradle.kts")
                            exclude(buildLogicOutputs)
                        }
                } else {
                    files("build.gradle.kts")
                }

            extensions.configure<SpotlessExtension> {
                kotlin {
                    target(writtenKotlin)
                    repositoryKtlint()
                }
                kotlinGradle {
                    target(writtenGradle)
                    repositoryKtlint()
                }
            }

            // The rules above are the plugin's last word only until the project's own build file runs
            // after it, where `spotless { kotlin { targetExclude("src/**") } }`, a narrowed `target`, or
            // disabling one of the tasks would quietly decide what gets formatted at all. So what each
            // Spotless task will really compare is read back from the task itself once the task graph is
            // ready, after every build file and every `afterEvaluate` or `projectsEvaluated` of theirs has
            // run: by then its `target` is the plugin's tree minus whatever a build file excluded, and a file
            // the convention formats and the project hides is named. (A check inside the task cannot do
            // this: an emptied target makes the task NO-SOURCE.) The read-back only demands coverage, so a
            // project may add files to format but not remove them.
            gradle.taskGraph.whenReady {
                val formats = listOf("kotlin" to writtenKotlin, "kotlinGradle" to writtenGradle)
                for ((format, written) in formats) {
                    val name = "spotless" + format.replaceFirstChar { it.uppercaseChar() }
                    val task = tasks.getByName(name) as SpotlessTask
                    val hidden = written.files - task.target.files
                    if (hidden.isNotEmpty()) {
                        throw GradleException(
                            "$path keeps ${hidden.joinToString { projectDir.toPath().relativize(it.toPath()).toString() }} " +
                                "out of $name, which craftbridge.quality formats in every module",
                        )
                    }
                }
                val disabled =
                    (
                        listOf("spotlessCheck") +
                            formats
                                .map { (format, _) ->
                                    val name = "spotless" + format.replaceFirstChar { it.uppercaseChar() }
                                    listOf(name, "${name}Check")
                                }.flatten()
                    ).filterNot { tasks.getByName(it).enabled }
                if (disabled.isNotEmpty()) {
                    throw GradleException("$path disables ${disabled.joinToString()}, which craftbridge.quality runs in every module")
                }
            }

            if (atRoot) {
                // The licence check needs what actually ships inside an AAR, and a module's build file
                // names at most the direct request; transitive modules appear only in Gradle's resolved
                // graph. So Gradle answers the question: one `group:name` per line, for `cargo xtask
                // rules` to match against tools/licences.toml. Resolution happens when the task runs,
                // so `cargo xtask rules` pays only for the modules that really land on a classpath.
                tasks.register("runtimeLicences") {
                    description =
                        "Print every external module resolved onto a library module's release, unit-test and instrumented-test runtime classpaths, one group:name per line."
                    group = "verification"
                    // The live dependency graph is read when the task runs and is not a serialisable
                    // task input, exactly as it is for Gradle's own `dependencies` report task.
                    notCompatibleWithConfigurationCache("reads the resolved dependency graph at execution time")
                    doLast {
                        val modules = sortedSetOf<String>()
                        rootProject.subprojects.filter { it.plugins.hasPlugin("com.android.library") }.forEach { library ->
                            // What ships, and what the tests run with: AGENTS §8 holds every dependency to
                            // the allowlist, as cargo-deny does a crate's dev-dependencies. A library
                            // without one of these classpaths would add nothing and look clean, so its
                            // absence is an error rather than an empty answer.
                            val classpaths =
                                licenceClasspaths.map { name ->
                                    library.configurations.findByName(name)
                                        ?: throw GradleException("${library.path} has no $name, so what it uses cannot be listed")
                                }
                            LicenceListing.addResolvedModules(classpaths, modules)
                        }
                        modules.forEach { println(it) }
                    }
                }
            }
        }
}
