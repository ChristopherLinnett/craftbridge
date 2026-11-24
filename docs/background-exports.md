# Background exports

An export of seconds ends before the platform cares where the app is. An export of minutes must survive the app leaving the foreground — and, on Android, the phone going to sleep mid-export. Both platforms have a recipe, the Postcard Studio samples implement both, and every Swift and Kotlin block here is a run of lines of the samples' own code, so the gate compiles what you read.

Where the export is short and the app stays open, the plain calls of the integration guides ([iOS](integration-ios.md), [Android](integration-android.md)) are enough.

## iOS: continued processing, and the journal

**iOS 26 and later.** The app starts the export from the user's tap inside a `BGContinuedProcessingTask`, reports CraftBridge's progress to it, and the system shows that progress and lets the export run when the app goes away. The launch handler must be registered before the app finishes launching — the sample does it like this:

```swift
static func registerBackgroundTask() {
    if #available(iOS 26.0, *) {
        ContinuedExportTask.register()
    }
}
```

For renders to keep the GPU in the background, the app carries the `com.apple.developer.background-tasks.continued-processing.gpu` entitlement, asks for the GPU in its task request (`requiredResources = .gpu`, on a phone whose `BGTaskScheduler.supportedResources` holds it), and starts its engine with `background: .keepGpu` (`EffectsEngine.start(folders:gpu:background:)`). Without both — and Postcard Studio, which carries no such entitlement, keeps the default — the engine gives its GPU up once the app enters the background: every render started after that answers on the CPU, while the export already running keeps the device it took until it ends. When the system wants its time back, the export must stop with grace — cancelling the job is what the sample does:

```swift
task?.onExpiry { job.cancel() }
```

and the task ends with the export, well or not:

```swift
do {
    _ = try await job.result
    task?.end(success: true)
} catch {
    task?.end(success: false)
    throw error
}
```

**iOS 17 to 25.** There is no continued processing task; the export continues for the short time iOS grants a backgrounded app, which `EffectsEngine` covers with a background task of its own for every export, on every version. Nothing is reported to the system; when the system is about to stop the app before the export ends, the engine cancels the export.

**The journal (every version, both platforms).** While an export runs, the engine keeps a journal of every file it is about to write. An export that is cancelled or fails deletes those files itself before it answers, so a file at an output path always means a finished render; the one exception is a queue that fails at one item or is cancelled, which keeps the files of the items it finished, as `renderQueue.list` reports them; a panic keeps none. A file the disk will not let go of stays in the journal, and the next `start` deletes it. When the *system* ends the process mid-export — its time ran out, or it was killed — the next `start` removes the half-written movie and reports what it removed:

```swift
let stopped = try engine.recovery().removed
```

The sample reads this at start and offers to export again: the movie the killed export was writing is gone, so the app's job after `start` is to offer a retry, not to look for the file.

## Android: a foreground service, and a wake lock beside it

Android ends a backgrounded app's work quickly; a foreground service is the sanctioned way to run minutes of work. Two things are needed and each does one job: the **foreground service** keeps the process alive, and a **partial wake lock** keeps the CPU awake for the export's length.

### The service

Declare it with both renderable types and the permissions each needs — the type chosen at start decides which permission answers for it:

```xml
<uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_DATA_SYNC" />
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_MEDIA_PROCESSING" />
<uses-permission android:name="android.permission.WAKE_LOCK" />
<uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
```

```xml
<service
    android:name=".ExportService"
    android:exported="false"
    android:foregroundServiceType="mediaProcessing|dataSync" />
```

Start the service from the user's tap, while the app is still in the foreground, and always give it a type:

```kotlin
startForeground(NOTIFICATION_ID, notification.showing(0uL, 0uL), foregroundServiceType())
```

```kotlin
private fun foregroundServiceType(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
        ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROCESSING
    } else {
        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
    }
```

`mediaProcessing` is the type built for media work and exists from Android 15 (`SDK_INT >= 35`); on Android 10–14 it does not — Android 14 rejects it, and a type-less start, so `dataSync` stands in until 15. Both types need their declaration in the Play Console, and from Android 15 the clock runs: when the type's time limit is spent Android calls `onTimeout`, and the service must stop within seconds — the sample treats the timeout exactly as the user's stop, whose end stops the service:

```kotlin
override fun onTimeout(
    startId: Int,
    fgsType: Int,
) {
    exporting?.cancel()
}
```

The service shows the export's progress in its notification (the sample re-notifies each report) and ends itself — `stopForeground` and `stopSelf` — however the export ends.

### The wake lock

A foreground service keeps the process alive; it does not keep the CPU awake. A phone off its charger with the screen off suspends the CPU in the middle of an export that holds nothing: the V5a measurements for issue #383 saw a phone asleep for up to 855 ms in one export, and its exports take up to 80 % longer than the same export kept awake; the baseline's own series of that phone ranges 1742–2206 ms with the phone left to sleep against 1741–1756 ms kept awake (`conformance/effects/baseline.toml`). So beside the service the app holds a **partial** wake lock — CPU awake, screen free to sleep — for exactly the export's length, released however the export ends. The sample keeps that rule in two small pieces (`ExportWakeLock.kt`): an interface and a combinator that takes and lets go around the export,

```kotlin
suspend fun <T> ExportLock.heldDuring(export: suspend () -> T): T {
    acquire()
    try {
        return export()
    } finally {
        release()
    }
}
```

and a `PartialWakeLock` taken with a backstop timeout far above any export's length, so even a wedged process frees the CPU. Every path through `heldDuring` lets go — returned, failed, or cancelled:

```kotlin
lock.heldDuring {
    movie.delete()
    studio.exportMovie(movie) { done, total, _ ->
        studio.exportIs(ExportState.Running(done, total))
        getSystemService(android.app.NotificationManager::class.java)
            .notify(NOTIFICATION_ID, notification.showing(done, total))
    }
}
```

Because the lock ends with the coroutine, the timeout path above needs nothing special: the cancellation that ends the export ends the lock with it.

### If the system kills anyway

A service is still killable — low memory ends even foreground services on small phones. The engine's journal is the same on Android: the next start of the engine removes the partial movie and reports it through `engine.core.recovery()`. Postcard Studio for Android does not read that report; it deletes the movie it is about to write before every export (`movie.delete()` above), and the user taps Export again.

## Cross-checks

On iOS the continued processing task is asked for at the user's tap, and on Android the service is started from the tap too: a request made once the app has left the foreground is refused by iOS, and by Android from Android 12. The engine's own contract is the same on both: `progress` is the only clock to trust, cancelling the job (or the coroutine or task that awaits it) is the only stop, and the movie at the output is whole once the export has answered — a stopped export's files are deleted as it stops, and a killed one's at the next start.
