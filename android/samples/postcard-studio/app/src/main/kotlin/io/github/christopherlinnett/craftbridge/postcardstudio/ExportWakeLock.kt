package io.github.christopherlinnett.craftbridge.postcardstudio

import android.os.PowerManager

/**
 * What an export holds so the phone does not sleep under it: a foreground service keeps the process alive, not
 * the CPU awake, and a phone off its charger with the screen off suspends the CPU in the middle of an export
 * that holds nothing.
 */
interface ExportLock {
    /** Takes the lock. */
    fun acquire()

    /** Lets the lock go; letting go of one that is not held is no error. */
    fun release()
}

/**
 * Runs [export] with this lock held for exactly its length: taken before it begins and released however it
 * ends — returned, failed, or cancelled — so no ending leaves the CPU held awake.
 */
suspend fun <T> ExportLock.heldDuring(export: suspend () -> T): T {
    acquire()
    try {
        return export()
    } finally {
        release()
    }
}

/**
 * A partial wake lock named [tag]. It is taken with a timeout as a backstop that frees the CPU even if the
 * process is wedged and never reaches [release]; the timeout is far above any export's length.
 */
class PartialWakeLock(
    power: PowerManager,
    tag: String,
    private val backstopMillis: Long = BACKSTOP_MILLIS,
) : ExportLock {
    // Not reference counted: one export holds it once, and a release always lets go.
    private val lock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, tag).apply { setReferenceCounted(false) }

    override fun acquire() = lock.acquire(backstopMillis)

    override fun release() {
        if (lock.isHeld) lock.release()
    }

    private companion object {
        /** Ten minutes: a 1080p card exports in seconds, and a long one far inside this. */
        const val BACKSTOP_MILLIS = 10 * 60 * 1000L
    }
}
