// The folders an engine starts in, laid out in an Android app's own data.

package io.github.christopherlinnett.craftbridge.core

import android.content.Context
import java.io.File
import io.github.christopherlinnett.craftbridge.ffi.core.Folders as BoundaryFolders

/**
 * The folders an engine names when it starts: the one folder it writes inside, the folders it only
 * reads, and the working folders it keeps inside the first.
 *
 * Every working folder lies inside [container], which is where everything an engine writes ends up —
 * an export's output included. The core refuses a start whose working folders it has to create lie
 * outside the container, and checks that each one can be written before it answers, so a folder on a
 * volume that will not take the write is refused at the start and not halfway through an export.
 * [appDefault] lays the folders out the way the core wants them; an app that wants its exports
 * somewhere else names its own paths.
 */
public data class Folders(
    /** The app's own folder: the engine's one read and write root, which it creates when it is missing. */
    public val container: String,
    /** The app's read-only folder — its unpacked assets, or none while it has none to unpack. */
    public val bundle: String?,
    /** Where the engine keeps its settings, inside the container. */
    public val config: String,
    /** Where the engine keeps what it can throw away, inside the container; the system may empty it. */
    public val cache: String,
    /** Where the engine keeps the models it has installed, inside the container. */
    public val models: String,
    /**
     * Where the engine keeps the copies it makes of sources it imports from folders it may not write,
     * inside the container. A saved project names what was copied here, so this is a folder the system
     * keeps and never one under the cache, which the system may empty.
     */
    public val staged: String,
    /** Where the app keeps its plug-ins, or none while they are switched off. */
    public val plugins: String?,
    /** Extra font folders, after the system's own. */
    public val fonts: List<String>,
) {
    /**
     * These folders as the native boundary takes them. A Craft app plugin passes them to the engine it
     * starts; an app hands [Folders] to that plugin and names no boundary of its own.
     */
    public fun toFfi(): BoundaryFolders =
        BoundaryFolders(
            container = container,
            bundle = bundle,
            config = config,
            cache = cache,
            models = models,
            staged = staged,
            plugins = plugins,
            fonts = fonts,
        )

    public companion object {
        /**
         * The folders of an app's own data, each in the Android folder the system keeps that kind of file
         * in: settings in the app's files, models and the copies of imported sources in the files that
         * are kept but not backed up — both can be many gigabytes, which no Android backup has room for,
         * and both the engine can do without — what can be thrown away in the app's cache, which the
         * system may empty at any time, and the container is the app's data folder, which holds all three,
         * so every working folder lies inside it as the core requires. The engine writes only inside the
         * container, so an export's output lies there too.
         *
         * Nothing else is named: plug-ins stay off until an app names the folder it keeps them in, since
         * whether to load them is the app's choice (PD13), and an app that unpacks its assets into its
         * files folder names that folder as the bundle when it does, since a bundle the engine cannot read
         * is refused at the start.
         */
        public fun appDefault(context: Context): Folders =
            appDefault(
                data = context.dataDir.path,
                files = context.filesDir.path,
                cache = context.cacheDir.path,
                noBackup = context.noBackupFilesDir.path,
            )

        /**
         * [appDefault] with the app's folders already taken from its [Context], so the layout is one
         * definition and can be read, and checked, without a device to ask for them.
         */
        internal fun appDefault(
            data: String,
            files: String,
            cache: String,
            noBackup: String,
        ): Folders {
            val kept = File(noBackup, OWN_FOLDER)
            return Folders(
                container = data,
                bundle = null,
                config = File(File(files, OWN_FOLDER), "config").path,
                cache = File(cache, OWN_FOLDER).path,
                models = File(kept, "models").path,
                staged = File(kept, "staged").path,
                plugins = null,
                fonts = emptyList(),
            )
        }
    }
}

/** The folder of CraftBridge's own inside each Android folder the app's files sit in. */
private const val OWN_FOLDER = "CraftBridge"
