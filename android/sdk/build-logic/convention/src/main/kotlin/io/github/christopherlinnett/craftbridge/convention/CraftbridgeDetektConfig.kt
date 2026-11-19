package io.github.christopherlinnett.craftbridge.convention

import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.TaskAction

/**
 * Writes the per-module part of the module's detekt config: the package its files must declare, which
 * the plugin derives from the module's own namespace — a module can no more change what it is checked
 * against than rename itself — the module's own folder, from which the JNI rule reads a file's source
 * set, and its generated folder, from which the naming rule tells the generated facade from written code.
 * Everything else detekt is told lives once, in the profiles under build-logic/detekt-rules/profiles,
 * which each run merges with one of these files.
 */
abstract class CraftbridgeDetektConfig : DefaultTask() {
    /** The package the module's written code declares: its namespace. */
    @get:Input
    abstract val writtenPackage: Property<String>

    /** The package the module's generated bindings declare: the namespace's `ffi` spelling. */
    @get:Input
    abstract val bindingsPackage: Property<String>

    /**
     * The module's folder, as detekt names the files it reads: real path, `/` separators. A file's
     * place in its source set is read from below this folder, so nothing above the module counts.
     */
    @get:Input
    abstract val moduleDirectory: Property<String>

    /** The module's generated Kotlin folder, named the same way: what sits below it the generator wrote. */
    @get:Input
    abstract val generatedFolder: Property<String>

    @get:OutputFile
    abstract val writtenConfig: RegularFileProperty

    @get:OutputFile
    abstract val bindingsConfig: RegularFileProperty

    @TaskAction
    fun write() {
        // A single-quoted YAML scalar holds any text once its own quote is doubled.
        fun quoted(value: String) = "'" + value.replace("'", "''") + "'"
        // The JNI and naming rules read only written code, so only the written file says where the module
        // and its generated folder are.
        val written =
            "  ExternalOutsidePlatformFolder:\n    moduleDirectory: ${quoted(moduleDirectory.get())}\n" +
                "  GeneratedNameTaken:\n    generatedFolder: ${quoted(generatedFolder.get())}\n"
        for ((file, expected, more) in listOf(
            Triple(writtenConfig, writtenPackage, written),
            Triple(bindingsConfig, bindingsPackage, ""),
        )) {
            file.get().asFile.apply {
                parentFile.mkdirs()
                writeText(
                    """
                    |# Written by the Android convention plugin from this module's namespace and folder; the
                    |# rules themselves live in android/sdk/build-logic/detekt-rules/profiles.
                    |# Validation is on, and leaves nothing out, whatever the profiles say: detekt merges the configs in
                    |# order and the last wins, and a run with validation off, or a property path excluded from it,
                    |# would let a misspelt or switched-off rule pass unseen.
                    |config:
                    |  validation: true
                    |  excludes: []
                    |craftbridge:
                    |  ModulePackageMismatch:
                    |    expectedPackages:
                    |      - ${quoted(expected.get())}
                    |
                    """.trimMargin() + more,
                )
            }
        }
    }
}
