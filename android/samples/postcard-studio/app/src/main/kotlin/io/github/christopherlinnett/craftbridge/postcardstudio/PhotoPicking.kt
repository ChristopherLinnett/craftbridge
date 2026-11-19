package io.github.christopherlinnett.craftbridge.postcardstudio

import android.content.Context
import android.net.Uri
import android.webkit.MimeTypeMap
import java.io.File
import java.io.FileNotFoundException

/** The folder the picked photo is kept in: the card has one Photo control, so the latest pick replaces the last. */
private const val PICKED_FOLDER = "picked"

/**
 * The picture at [picked], which the photo picker gave as a content URI, as a file in the app's own folder: the
 * engine reads files, never a URI, a picked photo's grant lasts only as long as the app is using it, and the
 * engine knows a picture by its file's extension, which the content's own type gives.
 *
 * @throws java.io.IOException when the picture cannot be read.
 */
fun Context.keepPickedPhoto(picked: Uri): File {
    val folder = File(filesDir, PICKED_FOLDER)
    folder.deleteRecursively()
    folder.mkdirs()
    val extension = contentResolver.getType(picked)?.let { MimeTypeMap.getSingleton().getExtensionFromMimeType(it) } ?: "jpg"
    // A new name for every pick, so the card sees a different picture even when the new one has the old one's type.
    val copy = File(folder, "photo-${System.nanoTime()}.$extension")
    val source = contentResolver.openInputStream(picked) ?: throw FileNotFoundException("the picker gave no picture at $picked")
    source.use { input -> copy.outputStream().use { output -> input.copyTo(output) } }
    return copy
}
