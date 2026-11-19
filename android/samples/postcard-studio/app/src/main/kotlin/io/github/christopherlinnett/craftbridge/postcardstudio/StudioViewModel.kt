package io.github.christopherlinnett.craftbridge.postcardstudio

import android.app.Application
import android.net.Uri
import android.view.Surface
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.christopherlinnett.craftbridge.core.CraftException
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.File
import java.io.IOException

/** The second of the card the preview shows to begin with. */
private const val FIRST_SECOND = 2.0

/** How long the playing preview waits between two frames: about thirty to the second, the card's own rate. */
private const val FRAME_MILLIS = 33L

/** What the screen shows and what Maya has chosen so far. */
data class StudioState(
    /** Whether the engine is still starting and the card opening. */
    val starting: Boolean = true,
    /** The name the card greets: the Name control's value. */
    val name: String = "Sam",
    /** The second of the card the preview shows. */
    val second: Double = FIRST_SECOND,
    /** Whether the preview plays the card through. */
    val playing: Boolean = false,
    /** A note under the preview: the photo's or the preview's own trouble, which stops neither. */
    val note: String? = null,
)

/** Every step Maya takes (plan §5): the card opens, she types a name, picks a photo, watches the preview, exports and shares. */
class StudioViewModel(
    application: Application,
) : AndroidViewModel(application) {
    private val studio = (application as PostcardStudioApplication).studio
    private val mutableState = MutableStateFlow(StudioState())
    private val changed = Channel<Unit>(Channel.CONFLATED)
    private var photo: File? = null
    private var appliedName: String? = null
    private var appliedPhoto: File? = null
    private var surface: Surface? = null
    private var player: Job? = null

    /** What the screen shows. */
    val state: StateFlow<StudioState> get() = mutableState

    /** Where the export is. */
    val export: StateFlow<ExportState> get() = studio.export

    init {
        viewModelScope.launch {
            noting { studio.start() }
            mutableState.update { it.copy(starting = false) }
            for (ignored in changed) refresh()
        }
        // The preview is drawn again when an export ends, since none is drawn while one runs.
        viewModelScope.launch {
            export.collect { if (it !is ExportState.Running) changed.trySend(Unit) }
        }
    }

    /** Sets the name the card greets. */
    fun nameIs(name: String) {
        mutableState.update { it.copy(name = name) }
        changed.trySend(Unit)
    }

    /** Moves the preview to [second] of the card. */
    fun secondIs(second: Double) {
        mutableState.update { it.copy(second = second) }
        changed.trySend(Unit)
    }

    /** Takes the picture the photo picker gave. */
    fun photoPicked(picked: Uri) {
        viewModelScope.launch { noting { photoIs(getApplication<Application>().keepPickedPhoto(picked)) } }
    }

    /** Takes the app's own sample photo, for trying the card without a picture of one's own. */
    fun samplePhotoChosen() {
        viewModelScope.launch { noting { photoIs(samplePhoto(getApplication())) } }
    }

    private fun photoIs(file: File) {
        photo = file
        changed.trySend(Unit)
    }

    /** The preview's view has a surface to draw into, or has lost it. */
    fun surfaceIs(surface: Surface?) {
        this.surface = surface
        if (surface == null) {
            player?.cancel()
            viewModelScope.launch { studio.endPreview() }
        } else {
            changed.trySend(Unit)
        }
    }

    /** Plays the preview through from the second it shows, or stops it. */
    fun togglePlaying() {
        if (mutableState.value.playing) {
            player?.cancel()
            return
        }
        mutableState.update { it.copy(playing = true) }
        player =
            viewModelScope.launch {
                try {
                    while (isActive) {
                        delay(FRAME_MILLIS)
                        val next = mutableState.value.second + FRAME_MILLIS / 1000.0
                        secondIs(if (next > CARD_SECONDS) 0.0 else next)
                    }
                } finally {
                    mutableState.update { it.copy(playing = false) }
                }
            }
    }

    /** Exports the card in the foreground service, which keeps going when the user leaves the app. */
    fun startExport() {
        player?.cancel()
        ExportService.start(getApplication())
    }

    /** Stops the export that is running. */
    fun stopExport() {
        ExportService.stop(getApplication())
    }

    /** Sets the controls that changed, then draws the preview at the second shown. A change made while this runs is the next turn's. */
    private suspend fun refresh() {
        if (export.value is ExportState.Running) return
        val shown = mutableState.value
        noting {
            if (appliedName != shown.name) {
                studio.setName(shown.name)
                appliedName = shown.name
            }
            photo?.takeIf { it != appliedPhoto }?.let {
                studio.setPhoto(it)
                appliedPhoto = it
            }
            surface?.takeIf { it.isValid }?.let { studio.preview(it, shown.second) }
        }
    }

    /**
     * Runs [step], putting what stops it in the note: the engine's own words, or the disk's when a file the card needs
     * cannot be written, as on a full disk. Neither stops the screen, and the next change tries again.
     */
    private suspend fun noting(step: suspend () -> Unit) {
        try {
            step()
        } catch (failure: CraftException) {
            mutableState.update { it.copy(note = failure.message) }
        } catch (failure: IOException) {
            mutableState.update { it.copy(note = failure.message) }
        }
    }

    override fun onCleared() {
        player?.cancel()
    }
}
