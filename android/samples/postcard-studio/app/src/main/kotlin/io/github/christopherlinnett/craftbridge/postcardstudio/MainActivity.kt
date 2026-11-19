package io.github.christopherlinnett.craftbridge.postcardstudio

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels

/** The app's one activity. */
class MainActivity : ComponentActivity() {
    private val studio: StudioViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val picker =
                rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { picked ->
                    picked?.let(studio::photoPicked)
                }
            StudioScreen(
                studio = studio,
                pickPhoto = { picker.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                startExport = {
                    askToShowProgress()
                    studio.startExport()
                },
            )
        }
    }

    /**
     * Asks, when the user first exports, to show the export's notification, which Android 13 and later keep from an app that has not been
     * allowed; the export itself runs whatever the answer.
     */
    private fun askToShowProgress() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
        }
    }
}
