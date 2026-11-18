// The FP16 floor of the Android arm64 library, met by an engine started through the SDK on a device.

package io.github.christopherlinnett.craftbridge.effects

import android.util.Log
import io.github.christopherlinnett.craftbridge.core.UnsupportedException
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The auxiliary vector's tag for the kernel's capability bits (Linux's `AT_HWCAP`). */
private const val AT_HWCAP = 16L

/** `HWCAP_FPHP` and `HWCAP_ASIMDHP` of Linux's `asm/hwcap.h` for arm64: half-precision scalar and vector arithmetic. */
private const val HWCAP_FPHP = 1L shl 9
private const val HWCAP_ASIMDHP = 1L shl 10

/**
 * The kernel's capability word as this process was started with it, read from `/proc/self/auxv` (pairs of
 * 64-bit tag and value, ended by a tag of zero), apart from the engine's own read through `getauxval`. `null`
 * where the kernel gave the process none.
 */
private fun capabilityWord(): Long? {
    val auxv = ByteBuffer.wrap(File("/proc/self/auxv").readBytes()).order(ByteOrder.LITTLE_ENDIAN)
    while (auxv.remaining() >= 2 * Long.SIZE_BYTES) {
        val tag = auxv.getLong()
        val value = auxv.getLong()
        if (tag == AT_HWCAP) {
            return value
        }
        if (tag == 0L) {
            break
        }
    }
    return null
}

/**
 * What the engine's start does about half-precision arithmetic: the arm64 library is built for it, so on an arm64
 * phone it starts where the kernel reports both capability bits and refuses, naming each one lacking, where it
 * does not; on a build with no half-precision path (the x86_64 emulator) there is no floor. The refusal is
 * covered on the host, through the core's capability seam, since no phone here lacks the bits.
 */
internal val fp16FloorChecks: List<Case> =
    listOf(
        Case("an engine started through the SDK meets the FP16 floor on an arm64 phone that reports both capability bits") { context ->
            val arm64 = System.getProperty("os.arch") == "aarch64"
            val word = if (arm64) capabilityWord() else null
            val bits = word ?: 0L
            val lacking =
                listOfNotNull(
                    "HWCAP_FPHP".takeIf { bits and HWCAP_FPHP == 0L },
                    "HWCAP_ASIMDHP".takeIf { bits and HWCAP_ASIMDHP == 0L },
                )
            Log.i(LOG_TAG, "arm64 $arm64, AT_HWCAP ${word?.let { "0x${it.toString(16)}" }}, lacking $lacking")
            val started = runCatching { startOn(context) }
            if (!arm64 || lacking.isEmpty()) {
                started.getOrThrow().shutDown()
            } else {
                val refusal = started.exceptionOrNull()
                require(refusal is UnsupportedException) { "an engine started on a phone lacking $lacking: $refusal" }
                require(lacking.all { refusal.message.orEmpty().contains(it) }) { "the refusal does not name $lacking: ${refusal.message}" }
            }
        },
    )
