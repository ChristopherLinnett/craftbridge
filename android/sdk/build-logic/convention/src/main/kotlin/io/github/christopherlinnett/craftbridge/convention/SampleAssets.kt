package io.github.christopherlinnett.craftbridge.convention

import com.android.build.api.variant.Component
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.Project
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.kotlin.dsl.register
import org.gradle.process.ExecOperations
import javax.inject.Inject

/** The script that makes the running example's template from its recipe: the one definition of how, which every sample bundles the result of. */
private const val TEMPLATE_SCRIPT = "tools/birthday-card.sh"

/**
 * What changes what the script makes: the recipe it replays and the placeholder photo it names, and which command line
 * replays it, the EffectCraft revision `rust/Cargo.toml` pins and `tools/oracle.sh` builds. A pin that moves makes the
 * template again, as the gate's own builds, which start from nothing, always do.
 */
private val TEMPLATE_INPUTS =
    listOf("conformance/effects/birthday-card.jsonl", "conformance/effects/fixtures/photo.png", "rust/Cargo.toml", "tools/oracle.sh")

/** The photo a sample offers for trying the card without a picture of the user's own: the corpus's own `sam.png`. */
private const val SAMPLE_PHOTO = "conformance/effects/fixtures/sam.png"

/**
 * The assets a sample bundles: the running example's template, made by the repository's script from the repository's
 * recipe, and the sample photo. Nothing is kept in the sample's tree, so the template is the recipe's and nothing else's.
 */
abstract class SampleAssets : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val script: RegularFileProperty

    /** The files the script reads. */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val scriptInputs: ConfigurableFileCollection

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NAME_ONLY)
    abstract val photo: RegularFileProperty

    @get:OutputDirectory
    abstract val assets: DirectoryProperty

    @get:Inject
    protected abstract val execOperations: ExecOperations

    @TaskAction
    fun make() {
        val folder = assets.get().asFile
        folder.deleteRecursively()
        folder.mkdirs()
        execOperations.exec { commandLine(script.get().asFile.path, folder.path) }
        val made = folder.resolve("Birthday Card.ectemplate")
        if (!made.isFile) throw GradleException("${script.get().asFile.name} made no template at $made")
        val sample = photo.get().asFile
        sample.copyTo(folder.resolve(sample.name), overwrite = true)
    }
}

/** Bundles the sample assets in [variant], by AGP's own generated-assets API. */
internal fun Project.bundleSampleAssets(variant: Component) {
    val make =
        tasks.register<SampleAssets>("make${variant.name.replaceFirstChar(Char::uppercaseChar)}SampleAssets") {
            description = "Makes the running example's template and bundles it, with the sample photo, in the ${variant.name} build"
            script.set(repositoryRoot.resolve(TEMPLATE_SCRIPT))
            scriptInputs.from(TEMPLATE_INPUTS.map { repositoryRoot.resolve(it) })
            photo.set(repositoryRoot.resolve(SAMPLE_PHOTO))
        }
    variant.sources.assets?.addGeneratedSourceDirectory(make, SampleAssets::assets)
}
