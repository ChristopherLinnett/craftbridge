// The publishing machinery of the two Maven artifacts (plan §23): the version the release script hands
// in, the repository it publishes to, what each artifact is, and which ABIs an artifact's native libraries
// are built for, which a consumer must be told from the metadata alone; that JNA's AAR, not its jar, is the
// file to take is said by the dependency's own selection, which the convention plugin makes. Like every
// shared build setup it is configured once, here, and no module file names it.

package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.dsl.LibraryExtension
import com.android.build.api.variant.LibraryAndroidComponentsExtension
import org.gradle.api.GradleException
import org.gradle.api.Project
import org.gradle.api.XmlProvider
import org.gradle.api.artifacts.DependencyArtifact
import org.gradle.api.artifacts.ExternalModuleDependency
import org.gradle.api.artifacts.ModuleIdentifier
import org.gradle.api.artifacts.dsl.DependencyHandler
import org.gradle.api.provider.Provider
import org.gradle.api.publish.PublishingExtension
import org.gradle.api.publish.maven.MavenPublication
import org.gradle.api.publish.maven.tasks.AbstractPublishToMaven
import org.gradle.kotlin.dsl.configure
import org.gradle.kotlin.dsl.getByType

/** Both artifacts are published under this group: `io.github.christopherlinnett.craftbridge:<module>` (AGENTS §9). */
private const val MAVEN_GROUP = "io.github.christopherlinnett.craftbridge"

/** The release script passes the version `rust/Cargo.toml` gives as this property (AGENTS §11, D-27);
 * nothing is written down here, and without the property every publish task refuses to run. */
private const val VERSION_PROPERTY = "craftbridge.version"

/** The repository §23 names: GitHub Packages, which the release runs to ChristopherLinnett's token. */
private const val GITHUB_PACKAGES_URL = "https://maven.pkg.github.com/ChristopherLinnett/craftbridge"

/** The home page of the source both artifacts come from. */
private const val PROJECT_URL = "https://github.com/ChristopherLinnett/craftbridge"

/** The one variant §23 publishes: the build apps link. One variant keeps the artifacts the two AARs they
 * are, rather than four. */
private const val PUBLISHED_VARIANT = "release"

/** The POM property that names the ABIs an artifact's native libraries are built for, for the consuming
 * app to keep among its filters. */
private const val ABI_PROPERTY = "craftbridge.abiFilters"

/**
 * What each published module is, as its POM tells a reader of the repository. A module that is not
 * here cannot publish: a description that fell back to another module's would say what it is not.
 */
private val descriptions =
    mapOf(
        "core" to
            "CraftBridge for Android: the universal core every Craft app plugin runs on — the engine's requests " +
            "as coroutines, their progress and cancellation, its errors and its folders.",
        "effects" to
            "CraftBridge for Android: the EffectCraft plugin — the engine's start, the fixed-shape calls, " +
            "templates and their controls, and the generated command namespaces.",
    )

/** The `@aar` notation's own artifact selector, built once outside the module files, which AGENTS §10
 * keeps free of notation: asking for the AAR is what the coordinate alone cannot say. */
internal fun DependencyHandler.aarSelectorOf(module: ModuleIdentifier): DependencyArtifact =
    (create("${module.group}:${module.name}@aar") as ExternalModuleDependency).artifacts.first()

/** JNA as the catalog names it once: the module whose AAR, beside the jar its POM names, carries the
 * native dispatch library. */
internal fun Project.jnaModule(): ModuleIdentifier = catalogLibrary("jna").get().module

internal fun Project.configurePublishing() {
    extensions.configure<LibraryExtension> {
        publishing {
            singleVariant(PUBLISHED_VARIANT)
        }
    }
    pluginManager.apply("maven-publish")
    val abis = packagedAbis()
    val release = providers.gradleProperty(VERSION_PROPERTY)
    // A publication with no version of its own takes the project's, which Gradle calls `unspecified`, and
    // publishes under it without a word; a publish is only ever of the version the release names.
    tasks.withType(AbstractPublishToMaven::class.java).configureEach {
        doFirst {
            if (!release.isPresent) {
                throw GradleException(
                    "$path: no `$VERSION_PROPERTY`: a publish takes the version rust/Cargo.toml gives, which the release passes in",
                )
            }
        }
    }
    // AGP names the release component when its variants are made, which is after this plugin has run.
    afterEvaluate {
        extensions.configure<PublishingExtension> {
            publications.create(PUBLISHED_VARIANT, MavenPublication::class.java) {
                groupId = MAVEN_GROUP
                release.orNull?.let { version = it }
                from(components.getByName(PUBLISHED_VARIANT))
                describePom(project.name, abis)
            }
            repositories {
                maven {
                    name = "GitHubPackages"
                    url = uri(GITHUB_PACKAGES_URL)
                    credentials {
                        username = providers.gradleProperty("gpr.user").orElse(providers.environmentVariable("GITHUB_ACTOR")).orNull
                        password = providers.gradleProperty("gpr.key").orElse(providers.environmentVariable("GITHUB_TOKEN")).orNull
                    }
                }
                // A folder named by this property takes a copy of the publication beside GitHub, which is
                // how a release run proves its own metadata before it publishes for real.
                providers.gradleProperty("craftbridge.localRepo").orNull?.let { folder ->
                    maven {
                        name = "LocalProof"
                        url = uri(folder)
                    }
                }
            }
        }
    }
}

/**
 * The ABIs the published variant packages a native library for: the folders of its `jniLibs` sources,
 * as AGP's variant API lists them, which is what the AAR's `jni/` holds. Read off what ships, not
 * written down again: the ABIs are the Android targets `rust/rust-toolchain.toml` names, which
 * `tools/build-android.sh` builds into those folders, so a list kept here would be a second copy of
 * that one. A module with no native library of its own answers none.
 */
private fun Project.packagedAbis(): Provider<List<String>> {
    val abis = objects.listProperty(String::class.java)
    val android = extensions.getByType<LibraryAndroidComponentsExtension>()
    android.onVariants(android.selector().withName(PUBLISHED_VARIANT)) { variant ->
        val folders = variant.sources.jniLibs?.all ?: return@onVariants
        abis.set(
            folders.map { layers ->
                layers
                    .flatten()
                    .flatMap { folder ->
                        folder.asFile
                            .listFiles()
                            .orEmpty()
                            .filter { abi -> abi.listFiles().orEmpty().any { it.isFile } }
                    }.map { it.name }
                    .distinct()
                    .sorted()
            },
        )
    }
    return abis
}

/** What the POM says this artifact is, and what its resolver must read off it without the source at hand. */
private fun MavenPublication.describePom(
    module: String,
    abis: Provider<List<String>>,
) {
    pom {
        name.set("CraftBridge $module")
        description.set(
            abis.map { built ->
                val what =
                    descriptions[module]
                        ?: throw GradleException("`:$module` has no description to publish under in build-logic's Publishing.kt")
                if (built.isEmpty()) what else "$what The native libraries are built for ${built.joinToString()}."
            },
        )
        url.set(PROJECT_URL)
        licenses {
            license {
                name.set("MIT License")
                url.set("https://opensource.org/licenses/MIT")
            }
            license {
                name.set("Apache License, Version 2.0")
                url.set("https://www.apache.org/licenses/LICENSE-2.0.txt")
            }
        }
        scm {
            connection.set("scm:git:$PROJECT_URL.git")
            developerConnection.set("scm:git:ssh://git@github.com/ChristopherLinnett/craftbridge.git")
            url.set(PROJECT_URL)
            tag.set(version)
        }
        withXml { writeAbisInto(this, abis.get()) }
    }
}

/**
 * The ABIs, as the POM's properties, which Gradle's own POM DSL has no field for. JNA's AAR needs nothing
 * here: the artifact selector the convention plugin puts on the declared dependency is what Gradle writes
 * both the POM's `type` and the module metadata's selector from.
 */
private fun writeAbisInto(
    facade: XmlProvider,
    abis: List<String>,
) {
    if (abis.isNotEmpty()) {
        val document = facade.asElement().ownerDocument
        val properties = document.createElement("properties")
        properties.appendChild(document.createElement(ABI_PROPERTY).also { it.textContent = abis.joinToString(",") })
        facade.asElement().appendChild(properties)
    }
}
