package com.example.fgslab

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import java.io.File
import java.io.RandomAccessFile
import kotlin.concurrent.thread
import kotlin.math.sqrt

/**
 * Simulates a long CraftBridge/EffectCraft export: CPU-bound loop appending to a
 * file in filesDir, progress in the notification. The default length (10 min) is
 * the shortest export the guide has to cover; it outlasts the 3-minute
 * shortService limit, while the 6-hour dataSync/mediaProcessing limits are
 * reached only by shortening them with adb (see the FGS timeout docs).
 *
 * The foreground service type to use is passed as [EXTRA_TYPE] so one APK can
 * test every scenario: none | dataSync | mediaProcessing | specialUse |
 * shortService | auto (auto = mediaProcessing on API 35+, dataSync below).
 */
class ExportService : Service() {

    companion object {
        const val TAG = "CraftFGS"
        const val NID = 4711
        const val CHANNEL_ID = "export"
        const val EXTRA_TYPE = "fgs_type"
        const val EXTRA_DURATION_MS = "fgs_duration_ms"
        const val DEFAULT_DURATION_MS = 600_000L
    }

    @Volatile private var stopWorker = false
    private var worker: Thread? = null
    private val notificationManager by lazy { getSystemService(NotificationManager::class.java) }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "SERVICE_CREATED sdk=${Build.VERSION.SDK_INT} targetSdk=${applicationInfo.targetSdkVersion}")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val type = intent?.getStringExtra(EXTRA_TYPE) ?: "none"
        val durationMs = intent?.getLongExtra(EXTRA_DURATION_MS, DEFAULT_DURATION_MS) ?: DEFAULT_DURATION_MS
        Log.i(TAG, "START_REQUESTED type=$type durationMs=$durationMs")
        startExport(type, durationMs)
        return START_NOT_STICKY
    }

    /** The recommendation under test: pick mediaProcessing on 15+ only. Null = no type argument. */
    private fun resolveType(name: String): Int? = when (name) {
        "none" -> null
        "dataSync" -> ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        "mediaProcessing" -> ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
        "specialUse" -> ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        "shortService" -> ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE
        "auto" -> if (Build.VERSION.SDK_INT >= 35) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
        } else {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
        }
        // Thrown outside the startForeground try block: a mistyped harness argument
        // must crash, not be logged as if the platform had rejected a type.
        else -> throw IllegalArgumentException("unknown type name: $name")
    }

    private fun startExport(typeName: String, durationMs: Long) {
        createChannel()
        val notification = buildNotification("Export starting ($typeName)")
        val typeBit = resolveType(typeName)
        try {
            if (typeBit == null) {
                startForeground(NID, notification)
            } else {
                startForeground(NID, notification, typeBit)
            }
            Log.i(TAG, "STARTFOREGROUND_OK requested=$typeName typeBit=${typeBit ?: 0}")
        } catch (e: Exception) {
            Log.e(TAG, "STARTFOREGROUND_FAIL requested=$typeName ex=${e.javaClass.name} msg=${e.message}")
            stopSelf()
            return
        }
        worker = thread(name = "export") { runExport(typeName, durationMs) }
    }

    private fun runExport(typeName: String, durationMs: Long) {
        val t0 = System.currentTimeMillis()
        val out = File(filesDir, "export.bin")
        try {
            RandomAccessFile(out, "rw").use { raf -> writeUntilDone(raf, out, t0, durationMs) }
        } catch (e: Exception) {
            // Without this the worker dies silently and the FGS stays up with no work.
            Log.e(TAG, "EXPORT_FAILED ex=${e.javaClass.name} msg=${e.message}")
            stopSelf()
            return
        }
        val elapsedS = (System.currentTimeMillis() - t0) / 1000
        if (stopWorker) {
            Log.i(TAG, "EXPORT_ABORTED elapsedS=$elapsedS")
        } else {
            Log.i(TAG, "EXPORT_DONE type=$typeName elapsedS=$elapsedS fileKiB=${out.length() / 1024}")
            stopSelf()
        }
    }

    private fun writeUntilDone(raf: RandomAccessFile, out: File, t0: Long, durationMs: Long) {
        raf.setLength(0)
        val chunk = ByteArray(65536)
        var acc = 0.0
        var lastNotifMs = -10_000L
        var lastMin = -1L
        while (!stopWorker) {
            val elapsed = System.currentTimeMillis() - t0
            if (elapsed >= durationMs) break
            // ~40 ms of CPU burn per tick, like a rendering frame.
            val spinUntil = System.currentTimeMillis() + 40
            while (System.currentTimeMillis() < spinUntil) {
                var i = 0
                while (i < 20000) {
                    acc += sqrt((i % 997).toDouble() + 1.0)
                    i++
                }
            }
            raf.write(chunk)
            if (elapsed - lastNotifMs >= 5000) {
                lastNotifMs = elapsed
                notificationManager.notify(
                    NID,
                    buildNotification("Exporting ${elapsed / 1000}s / ${durationMs / 1000}s (${out.length() / 1024} KiB)")
                )
            }
            val min = elapsed / 60000
            if (min != lastMin) {
                lastMin = min
                Log.i(TAG, "TICK elapsedS=${elapsed / 1000} fileKiB=${out.length() / 1024}")
            }
            Thread.sleep(60)
        }
    }

    /** Android 14 (API 34): shortService timeout callback. */
    override fun onTimeout(startId: Int) {
        Log.e(TAG, "FGS_ON_TIMEOUT_1ARG startId=$startId (shortService timeout, API 34 callback)")
        stopSelf()
    }

    /** Android 15 (API 35): timeout of any time-limited type; fgsType is the type that ran out. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.e(TAG, "FGS_ON_TIMEOUT_2ARG startId=$startId fgsType=$fgsType")
        stopSelf()
    }

    override fun onDestroy() {
        Log.i(TAG, "SERVICE_DESTROYED")
        stopWorker = true
        worker?.join(1_000)
        super.onDestroy()
    }

    private fun buildNotification(text: String): Notification =
        Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("FGS Lab export")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()

    private fun createChannel() {
        if (notificationManager.getNotificationChannel(CHANNEL_ID) == null) {
            notificationManager.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Export", NotificationManager.IMPORTANCE_LOW)
            )
        }
    }
}
