package io.github.christopherlinnett.craftbridge.postcardstudio

import android.content.Context
import java.io.File
import java.io.IOException

/** The name the app bundles the Birthday Card under, in its assets. */
private const val TEMPLATE_ASSET = "Birthday Card.ectemplate"

/** The name the app bundles its sample photo under, in its assets. */
private const val SAMPLE_PHOTO_ASSET = "sam.png"

/**
 * An asset as a file in the app's own folder [folder]. The engine reads files, not asset streams, so what the
 * app bundles is copied out before it is used: each time it is asked for, so an update's new bundle is the one
 * read, and through a file of its own moved into place, so a copy cut short by a full disk or by the process
 * ending is never left where the engine would read it.
 *
 * @throws IOException when the copy cannot be written.
 */
private fun Context.assetAsFile(
    asset: String,
    folder: String,
): File {
    val home = File(filesDir, folder)
    if (!home.isDirectory && !home.mkdirs()) throw IOException("cannot make the folder $home")
    val copy = File(home, asset)
    val part = File.createTempFile("$asset.", ".part", home)
    try {
        assets.open(asset).use { source -> part.outputStream().use { sink -> source.copyTo(sink) } }
        if (!part.renameTo(copy)) throw IOException("cannot put $copy in place")
    } finally {
        part.delete()
    }
    return copy
}

/**
 * The Birthday Card template, as a file the engine opens.
 *
 * @throws IOException when it cannot be copied out of the app.
 */
fun birthdayCardTemplate(context: Context): File = context.assetAsFile(TEMPLATE_ASSET, "templates")

/**
 * The photo the app offers for trying the card without a picture of the user's own.
 *
 * @throws IOException when it cannot be copied out of the app.
 */
fun samplePhoto(context: Context): File = context.assetAsFile(SAMPLE_PHOTO_ASSET, "photos")
