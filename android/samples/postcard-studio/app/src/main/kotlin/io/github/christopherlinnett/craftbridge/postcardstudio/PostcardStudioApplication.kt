package io.github.christopherlinnett.craftbridge.postcardstudio

import android.app.Application

/**
 * The app's process: it owns the one [CardStudio], which the screen and the export service both reach, since
 * the export must keep running when the screen's view model is gone and both talk to the same engine.
 */
class PostcardStudioApplication : Application() {
    val studio: CardStudio by lazy { CardStudio(this) }
}
