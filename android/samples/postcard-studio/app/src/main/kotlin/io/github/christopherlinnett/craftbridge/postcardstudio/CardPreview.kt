package io.github.christopherlinnett.craftbridge.postcardstudio

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView

/**
 * The card's live preview: a [SurfaceView], whose surface the engine draws into on the GPU, so the frames never
 * pass through the app's own code. [surfaceIs] hears the surface when it exists and `null` when it is gone.
 */
@Composable
fun CardPreview(
    surfaceIs: (android.view.Surface?) -> Unit,
    modifier: Modifier = Modifier,
) {
    AndroidView(
        modifier = modifier,
        factory = { context ->
            SurfaceView(context).apply {
                holder.addCallback(
                    object : SurfaceHolder.Callback {
                        override fun surfaceCreated(holder: SurfaceHolder) = surfaceIs(holder.surface)

                        override fun surfaceChanged(
                            holder: SurfaceHolder,
                            format: Int,
                            width: Int,
                            height: Int,
                        ) = surfaceIs(holder.surface)

                        override fun surfaceDestroyed(holder: SurfaceHolder) = surfaceIs(null)
                    },
                )
            }
        },
    )
}
