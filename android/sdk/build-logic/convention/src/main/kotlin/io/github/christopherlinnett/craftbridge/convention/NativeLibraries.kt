// The refusal of an AAR that ships no engine (plan §23): the module that packages the SDK's native library
// may hand a repository no AAR whose `jni/` lacks that library for some Android ABI, because an app that
// installs one opens to nothing — Android finds no library to load and the engine never starts. What the
// AAR must hold is no list kept here: `tools/aar-native-libraries.sh` asks `cargo xtask
// aar-native-libraries`, which reads the module, the library's file and the ABIs from the layer table and
// the toolchain file, and the release asks the same command of the AAR that reached the proof repository,
// so a publish run by hand and a release are refused for one reason, in one wording. Like every shared
// build setup it is configured once, here, and no module file names it.

package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.artifact.SingleArtifact
import com.android.build.api.variant.LibraryAndroidComponentsExtension
import org.gradle.api.DefaultTask
import org.gradle.api.Project
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.publish.maven.tasks.AbstractPublishToMaven
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.kotlin.dsl.configure
import org.gradle.kotlin.dsl.register
import org.gradle.process.ExecOperations
import javax.inject.Inject

/** The script that asks the check of one AAR; it builds no library, it only reads what a build left. */
private const val CHECK_SCRIPT = "tools/aar-native-libraries.sh"

/**
 * The reading of an AAR's entries: the JDK's own `jar`, run by the script, which lists what an AAR holds
 * exactly as the platform that unpacks it sees it. The task hands over the two things only the module can
 * say — which module this AAR was published by, and which file it is — and the script answers for both.
 */
abstract class NativeLibrariesCheck : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val script: RegularFileProperty

    /** The module whose AAR this is, as the publication names it: the check answers for a module, not a path. */
    @get:Input
    abstract val module: Property<String>

    /** The AAR the build made of the published variant, which is the file the publication hands over. */
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val archive: RegularFileProperty

    @get:Inject
    protected abstract val execOperations: ExecOperations

    @TaskAction
    fun check() {
        execOperations.exec { commandLine(script.get().asFile.path, module.get(), archive.get().asFile.path) }
    }
}

/**
 * Refuses this module's published AAR before it reaches a repository unless it holds the SDK's native
 * library for every ABI the toolchain names. Every module of the SDK is asked alike, because which one
 * packages a native library is the layer table's answer and no build file's: a module that ships none, the
 * core among them, is answered by the check in the same run and published as it always was.
 */
internal fun Project.configureNativeLibraryCheck() {
    val check =
        tasks.register<NativeLibrariesCheck>("checkNativeLibraries") {
            description = "Refuses the AAR of the variant this build publishes unless it holds the native library of every Android ABI"
            script.set(repositoryRoot.resolve(CHECK_SCRIPT))
            module.set(project.name)
        }
    // AGP hands over the AAR of the published variant as the artifact it builds; nothing here guesses a
    // file name or a folder, and a variant never published is never asked.
    extensions.configure<LibraryAndroidComponentsExtension> {
        onVariants(selector().withName(PUBLISHED_VARIANT)) { variant ->
            check.configure { archive.set(variant.artifacts.get(SingleArtifact.AAR)) }
        }
    }
    // The publication's own step, wherever it publishes: a hand-run publish is refused as the release is,
    // and no property switches the refusal off.
    tasks.withType(AbstractPublishToMaven::class.java).configureEach { dependsOn(check) }
}
