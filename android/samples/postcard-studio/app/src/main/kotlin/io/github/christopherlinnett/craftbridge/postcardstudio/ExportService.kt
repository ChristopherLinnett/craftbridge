package io.github.christopherlinnett.craftbridge.postcardstudio

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import io.github.christopherlinnett.craftbridge.core.CraftException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import java.io.IOException

/**
 * The card's export, run as the background recipe for Android says: a foreground service, started from the user's
 * tap while the app is in the foreground and always given its type, which keeps the process alive when the user
 * leaves the app, and a partial wake lock held for the export's length, which keeps the CPU awake when the screen
 * is off and the phone is off its charger. The lock is released however the export ends.
 */
class ExportService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var exporting: Job? = null
    private lateinit var lock: ExportLock

    override fun onCreate() {
        super.onCreate()
        lock = PartialWakeLock(getSystemService(PowerManager::class.java), WAKE_LOCK_TAG)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        when (intent?.action) {
            ACTION_STOP -> exporting?.cancel()
            else -> if (exporting?.isActive != true) begin()
        }
        return START_NOT_STICKY
    }

    /**
     * Android 15 gives a type of foreground service a time limit and calls this when it is spent: the export stops, as the
     * user's stop does.
     */
    override fun onTimeout(
        startId: Int,
        fgsType: Int,
    ) {
        exporting?.cancel()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun begin() {
        val notification = ExportNotification(this)
        startForeground(NOTIFICATION_ID, notification.showing(0uL, 0uL), foregroundServiceType())
        val studio = (application as PostcardStudioApplication).studio
        val movie = cardMovie()
        studio.exportIs(ExportState.Running(0uL, 0uL))
        exporting =
            scope.launch {
                try {
                    lock.heldDuring {
                        movie.delete()
                        studio.exportMovie(movie) { done, total, _ ->
                            studio.exportIs(ExportState.Running(done, total))
                            getSystemService(android.app.NotificationManager::class.java)
                                .notify(NOTIFICATION_ID, notification.showing(done, total))
                        }
                    }
                    studio.exportIs(ExportState.Finished(movie))
                } catch (stopped: CancellationException) {
                    studio.exportIs(ExportState.Stopped)
                    throw stopped
                } catch (failure: CraftException) {
                    studio.exportIs(ExportState.Failed(failure.message.orEmpty()))
                } catch (failure: IOException) {
                    // The template could not be copied out of the app for the engine to open, as on a full disk.
                    studio.exportIs(ExportState.Failed(failure.message.orEmpty()))
                } finally {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                }
            }
    }

    /**
     * The type the service is started under, always given: `mediaProcessing`, the type built for media work, from
     * Android 15, and `dataSync` before it, where Android 14 rejects `mediaProcessing` and a start with no type.
     */
    private fun foregroundServiceType(): Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
        } else {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        }

    companion object {
        /** Stops the export in progress. */
        const val ACTION_STOP = "io.github.christopherlinnett.craftbridge.postcardstudio.STOP"

        private const val NOTIFICATION_ID = 1
        private const val WAKE_LOCK_TAG = "postcardstudio:export"

        /** Starts the export of the card, from the user's tap. */
        fun start(context: Context) {
            context.startForegroundService(Intent(context, ExportService::class.java))
        }

        /** Stops the export in progress. */
        fun stop(context: Context) {
            context.startService(Intent(context, ExportService::class.java).setAction(ACTION_STOP))
        }
    }
}
