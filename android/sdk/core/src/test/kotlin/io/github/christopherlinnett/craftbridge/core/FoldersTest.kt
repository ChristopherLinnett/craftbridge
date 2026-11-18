// Where an app's engine folders go, and what the core will not accept.

package io.github.christopherlinnett.craftbridge.core

import java.io.File
import java.nio.file.Paths
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class FoldersTest {
    private val data = File("/data/user/0/com.example.app")

    private val folders =
        Folders.appDefault(
            data = data.path,
            files = File(data, "files").path,
            cache = File(data, "cache").path,
            noBackup = File(data, "no_backup").path,
        )

    /** The working folders the core creates and writes in: the ones it must find inside the container. */
    private val working = listOf(folders.config, folders.cache, folders.models, folders.staged)

    @Test
    fun theContainerIsTheAppDataFolderThatHoldsEveryWorkingFolder() {
        assertEquals(data.path, folders.container)
        for (folder in working) {
            assertTrue(Paths.get(folder).startsWith(Paths.get(folders.container)), "$folder lies outside the container")
            assertTrue(
                Paths.get(folder) != Paths.get(folders.container),
                "$folder is the container itself: the engine writes in each of them",
            )
        }
        assertEquals(working.distinct().size, working.size, "the working folders are each a folder of their own: $working")
    }

    @Test
    fun stagedSitsBesideTheFoldersTheSystemKeepsAndNeverUnderTheCache() {
        val cache = Paths.get(folders.cache)
        assertFalse(Paths.get(folders.staged).startsWith(cache), "${folders.staged} is under the cache, which the system may empty")
        assertEquals(
            Paths.get(folders.models).parent,
            Paths.get(folders.staged).parent,
            "${folders.staged} is not beside the models folder",
        )
    }

    @Test
    fun eachFolderIsInTheAndroidFolderThatKeepsItsKindOfFile() {
        assertTrue(
            Paths.get(folders.config).startsWith(File(data, "files").toPath()),
            "${folders.config} is not among the app's kept files",
        )
        assertTrue(
            Paths.get(folders.cache).startsWith(File(data, "cache").toPath()),
            "${folders.cache} is not in the cache the system may empty",
        )
        for (folder in listOf(folders.models, folders.staged)) {
            assertTrue(
                Paths.get(folder).startsWith(File(data, "no_backup").toPath()),
                "$folder is backed up: it can be far larger than an Android backup holds",
            )
        }
    }

    @Test
    fun nothingThatIsNotThereIsNamed() {
        assertNull(folders.bundle, "an app names its unpacked assets when it has unpacked any")
        assertNull(folders.plugins, "plug-ins stay off until the app names the folder it keeps them in")
        assertEquals(emptyList<String>(), folders.fonts, "the system's own fonts are all the engine is given")
    }

    @Test
    fun theBoundaryIsHandedEveryFolderOfItsOwn() {
        val boundary = folders.toFfi()
        assertEquals(
            listOf(
                boundary.container,
                boundary.bundle,
                boundary.config,
                boundary.cache,
                boundary.models,
                boundary.staged,
                boundary.plugins,
                boundary.fonts,
            ),
            listOf(
                folders.container,
                folders.bundle,
                folders.config,
                folders.cache,
                folders.models,
                folders.staged,
                folders.plugins,
                folders.fonts,
            ),
        )
    }
}
