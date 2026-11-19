package io.github.christopherlinnett.craftbridge.postcardstudio

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

private const val CHANNEL_ID = "export"

/** The notification the foreground service shows while the card exports: its progress, and a way to stop it. */
class ExportNotification(
    private val context: Context,
) {
    init {
        val channel = NotificationChannel(CHANNEL_ID, context.getString(R.string.export_channel), NotificationManager.IMPORTANCE_LOW)
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    /** The notification for an export that has made [done] of [total] frames; the bar is open-ended until the engine says [total]. */
    fun showing(
        done: ULong,
        total: ULong,
    ): Notification {
        val stop = Intent(context, ExportService::class.java).setAction(ExportService.ACTION_STOP)
        val stopIntent = PendingIntent.getService(context, 0, stop, PendingIntent.FLAG_IMMUTABLE)
        return Notification
            .Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_export)
            .setContentTitle(context.getString(R.string.exporting_title))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setProgress(total.toInt(), done.toInt(), total == 0uL)
            .addAction(Notification.Action.Builder(null, context.getString(R.string.stop), stopIntent).build())
            .build()
    }
}
