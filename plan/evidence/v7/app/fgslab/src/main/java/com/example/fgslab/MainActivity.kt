package com.example.fgslab

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.widget.TextView

/**
 * Launched by `adb shell am start` (stands in for the user tap, i.e. the app is
 * in the foreground when the service is started). Extras choose the scenario,
 * then the test presses HOME to background the app.
 */
class MainActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val tv = TextView(this)
        tv.textSize = 18f
        val pad = (16 * resources.displayMetrics.density).toInt()
        tv.setPadding(pad, pad, pad, pad)

        val type = intent?.getStringExtra(ExportService.EXTRA_TYPE)
        val durMs = intent?.getLongExtra(ExportService.EXTRA_DURATION_MS, ExportService.DEFAULT_DURATION_MS)
            ?: ExportService.DEFAULT_DURATION_MS
        if (type != null) {
            tv.text = "Started export ($type, ${durMs / 1000}s). Press HOME to background the app."
            val si = Intent(this, ExportService::class.java)
                .putExtra(ExportService.EXTRA_TYPE, type)
                .putExtra(ExportService.EXTRA_DURATION_MS, durMs)
            startForegroundService(si)
            Log.i(ExportService.TAG, "ACTIVITY_STARTED_FGS_FROM_FOREGROUND type=$type durationMs=$durMs sdk=${Build.VERSION.SDK_INT}")
        } else {
            tv.text = "FGS Lab: launch with --es ${ExportService.EXTRA_TYPE} <none|dataSync|mediaProcessing|specialUse|shortService|auto> --el ${ExportService.EXTRA_DURATION_MS} <ms>"
        }
        setContentView(tv)
    }
}
