// The app's own folders, against the core that has to be able to write in them.

package io.github.christopherlinnett.craftbridge.effects

import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.effects.startEffectsEngine
import java.io.File

/**
 * What a start through the boundary proves about [Folders.appDefault]: that the container it picks
 * holds every folder the engine writes in, that each of them is there afterwards, and that a folder
 * put outside the container is refused before anything is written. `GpuChoice.PREFERRED` is the
 * choice an app makes, and the machine is welcome to answer that it has no device to give.
 */
internal val folderChecks: List<Case> =
    listOf(
        Case("the app's own folders take an engine") { context ->
            val folders = Folders.appDefault(context)
            onEngine(context, GpuChoice.PREFERRED) { }
            for (folder in listOf(folders.config, folders.cache, folders.models, folders.staged)) {
                require(File(folder).isDirectory) { "$folder was not made: the start never reached the write there" }
            }
        },
        Case("a working folder outside the container is refused") { context ->
            val folders = Folders.appDefault(context)
            // A folder of the app's own that lies outside the container it names: the engine may write
            // in it, which is exactly what makes it the wrong place for a working folder.
            val outside = File(context.getExternalFilesDir(null), "craftbridge-outside").apply { mkdirs() }
            // The words of the refusal are what this check is after. Which exception the refusal reaches
            // an app as is the start's business, and the start that names them is the plugin's own.
            val refused =
                runCatching { startEffectsEngine(folders.copy(config = outside.path).toFfi(), GpuChoice.CPU_ONLY) }.exceptionOrNull()
            require(refused?.message.orEmpty().contains("is outside the container")) {
                "a config folder outside the container was answered ${refused ?: "nothing at all"}, not refused for lying outside"
            }
        },
    )
