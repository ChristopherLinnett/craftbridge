package io.github.christopherlinnett.craftbridge.postcardstudio

import android.content.Context
import android.util.Log
import android.view.Surface
import io.github.christopherlinnett.craftbridge.core.CraftException
import io.github.christopherlinnett.craftbridge.core.JSONValue
import io.github.christopherlinnett.craftbridge.core.ProgressHandler
import io.github.christopherlinnett.craftbridge.core.jsonValue
import io.github.christopherlinnett.craftbridge.effects.EffectsEngine
import io.github.christopherlinnett.craftbridge.effects.TemplateInstance
import io.github.christopherlinnett.craftbridge.effects.open
import io.github.christopherlinnett.craftbridge.effects.templates
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.time.Duration.Companion.seconds

/** The seconds the Birthday Card lasts, which the preview's time runs over. */
const val CARD_SECONDS = 6.0

/** The tag the app's own lines in the system log carry. */
private const val LOG_TAG = "PostcardStudio"

/** The card's two controls, as its template names them. */
private const val NAME_CONTROL = "Name"
private const val PHOTO_CONTROL = "Photo"

/** The engine and the card opened in it: every call the app makes into the SDK is made here. */
class CardStudio(
    private val context: Context,
) {
    /** The app's one engine and the Birthday Card opened in it, made by the first call that needs them. */
    private val opened =
        OpenedOnce(
            template = { birthdayCardTemplate(context) },
            start = { EffectsEngine.start(context) },
            open = { engine: EffectsEngine, template: File -> engine.templates.open(template.path) },
            shutDown = { engine: EffectsEngine -> engine.shutDown() },
            onEndFailure = { _: EffectsEngine, failure: Exception ->
                Log.w(LOG_TAG, "an engine whose card did not open could not be ended", failure)
            },
        )
    private val exportState = MutableStateFlow<ExportState>(ExportState.Idle)

    /** Where the export is. The export service writes it and the screen and the notification read it. */
    val export: StateFlow<ExportState> get() = exportState

    /** Records where the export is. */
    fun exportIs(state: ExportState) {
        exportState.value = state
    }

    /**
     * The card, opened once: the template the app bundles is copied out for the engine to read, then the engine
     * starts over the app's own folders asking for the GPU, and the template opens in it ([OpenedOnce]). The copy
     * and the start wait on the disk and the engine, so they run on the IO threads, never the caller's: the screen
     * asks for its first preview from the main thread, which must not wait for either.
     *
     * @throws java.io.IOException when the template cannot be copied out of the app.
     * @throws CraftException when the engine does not start or the card does not open.
     */
    private suspend fun card(): Opened<EffectsEngine, TemplateInstance> = withContext(Dispatchers.IO) { opened.get() }

    /** Opens the card, so the first preview does not wait for the engine to start. */
    suspend fun start() {
        card()
    }

    /** Puts [name] on the card's Name control. */
    suspend fun setName(name: String) {
        card().card.set(NAME_CONTROL, name.jsonValue)
    }

    /** Puts the picture at [photo] on the card's Photo control. */
    suspend fun setPhoto(photo: File) {
        card().card.set(PHOTO_CONTROL, photo)
    }

    /** Draws the card at [second] into [surface], which the engine keeps drawing into until [endPreview]. */
    suspend fun preview(
        surface: Surface,
        second: Double,
    ) {
        val studio = card()
        studio.engine.preview(surface, comp = studio.card.comp, time = second.seconds)
    }

    /** Lets the preview's surface go, when its view is. */
    suspend fun endPreview() {
        opened.engine?.endPreview()
    }

    /**
     * Exports every frame of the card as an H.264 movie at [movie], reporting each step of it to [onProgress].
     * Cancelling the calling coroutine stops the export the engine's own way, which leaves no half-written file.
     *
     * @throws CraftException when the engine refuses or fails the export.
     * @throws CancellationException when the export is cancelled.
     */
    suspend fun exportMovie(
        movie: File,
        onProgress: ProgressHandler,
    ) {
        val studio = card()
        movie.parentFile?.mkdirs()
        studio.engine.export(
            request =
                mapOf<String, JSONValue>(
                    "comp" to studio.card.comp,
                    "output" to movie.path.jsonValue,
                    "format" to "h264".jsonValue,
                ),
            onProgress = onProgress,
        )
    }
}
