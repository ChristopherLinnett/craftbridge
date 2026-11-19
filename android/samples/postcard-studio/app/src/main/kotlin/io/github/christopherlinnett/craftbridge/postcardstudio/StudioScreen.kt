package io.github.christopherlinnett.craftbridge.postcardstudio

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

/** Postcard Studio's one screen: the card's controls, its live preview, and the export with its progress and share. */
@Composable
fun StudioScreen(
    studio: StudioViewModel,
    pickPhoto: () -> Unit,
    startExport: () -> Unit,
) {
    val state by studio.state.collectAsState()
    val export by studio.export.collectAsState()
    val context = LocalContext.current
    val exporting = export is ExportState.Running
    MaterialTheme {
        Column(
            modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OutlinedTextField(
                value = state.name,
                onValueChange = studio::nameIs,
                label = { Text(stringResource(R.string.name_label)) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = pickPhoto, enabled = !state.starting) { Text(stringResource(R.string.pick_photo)) }
                OutlinedButton(onClick = studio::samplePhotoChosen, enabled = !state.starting) {
                    Text(stringResource(R.string.use_sample_photo))
                }
            }
            CardPreview(surfaceIs = studio::surfaceIs, modifier = Modifier.fillMaxWidth().aspectRatio(1f))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = studio::togglePlaying, enabled = !state.starting && !exporting) {
                    Text(stringResource(if (state.playing) R.string.pause else R.string.play))
                }
                Slider(
                    value = state.second.toFloat(),
                    onValueChange = { studio.secondIs(it.toDouble()) },
                    valueRange = 0f..CARD_SECONDS.toFloat(),
                    enabled = !state.starting && !exporting,
                    modifier = Modifier.weight(1f),
                )
            }
            ExportSection(
                export = export,
                ready = !state.starting,
                start = startExport,
                stop = studio::stopExport,
                share = { movie -> context.startActivity(context.shareIntent(movie)) },
            )
            state.note?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}

/** The export's button, its progress bar while it runs, its outcome, and the share of the movie it made. */
@Composable
private fun ExportSection(
    export: ExportState,
    ready: Boolean,
    start: () -> Unit,
    stop: () -> Unit,
    share: (java.io.File) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        when (export) {
            is ExportState.Running -> {
                LinearProgressIndicator(progress = { export.fraction }, modifier = Modifier.fillMaxWidth())
                Text(stringResource(R.string.exported_frames, export.done.toLong(), export.total.toLong()))
                OutlinedButton(onClick = stop) { Text(stringResource(R.string.stop)) }
            }

            is ExportState.Finished -> {
                Text(stringResource(R.string.exported))
                Button(onClick = { share(export.movie) }) { Text(stringResource(R.string.share)) }
                OutlinedButton(onClick = start) { Text(stringResource(R.string.export_again)) }
            }

            is ExportState.Failed -> {
                Text(export.reason, color = MaterialTheme.colorScheme.error)
                Button(onClick = start) { Text(stringResource(R.string.export_again)) }
            }

            ExportState.Stopped -> {
                Text(stringResource(R.string.stopped))
                Button(onClick = start) { Text(stringResource(R.string.export_again)) }
            }

            ExportState.Idle -> {
                Button(onClick = start, enabled = ready) { Text(stringResource(R.string.export)) }
            }
        }
    }
}
