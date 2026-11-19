package io.github.christopherlinnett.craftbridge.postcardstudio

import android.content.Context
import android.content.Intent
import androidx.core.content.FileProvider
import java.io.File

/** The folder of the app's cache the exported movies are written to, which the share provider lets other apps read from. */
private const val MOVIES_FOLDER = "movies"

/** The authority of the provider that lets another app read a movie, which the manifest declares. */
private fun Context.shareAuthority() = "$packageName.share"

/**
 * Where the card's movie is written: named for the card, never for the Name typed, which is free text a file name
 * cannot always hold, too long for one or holding a character the disk refuses, and the export would fail on it.
 */
fun Context.cardMovie(): File = File(File(cacheDir, MOVIES_FOLDER), "Birthday Card.mp4")

/** The system's offer to send [movie] to another app, which reads it through the share provider and never from the app's own folder. */
fun Context.shareIntent(movie: File): Intent {
    val uri = FileProvider.getUriForFile(this, shareAuthority(), movie)
    val send =
        Intent(Intent.ACTION_SEND)
            .setType("video/mp4")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    return Intent.createChooser(send, null)
}
