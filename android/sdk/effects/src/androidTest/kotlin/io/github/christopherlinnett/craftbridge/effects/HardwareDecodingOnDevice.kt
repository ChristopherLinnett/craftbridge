// The device's codec list, from the platform through the SDK to the engine's registration of its hardware decoders.

package io.github.christopherlinnett.craftbridge.effects

import android.media.MediaCodecList
import android.media.MediaFormat
import io.github.christopherlinnett.craftbridge.core.Engine
import io.github.christopherlinnett.craftbridge.core.Folders
import io.github.christopherlinnett.craftbridge.ffi.core.GpuChoice
import io.github.christopherlinnett.craftbridge.ffi.core.setDeviceCodecs
import io.github.christopherlinnett.craftbridge.ffi.effects.startEffectsEngine
import org.json.JSONObject

/** The types the engine's hardware decoder takes: H.264, HEVC and AV1. */
private val DECODED_TYPES = setOf(MediaFormat.MIMETYPE_VIDEO_AVC, MediaFormat.MIMETYPE_VIDEO_HEVC, MediaFormat.MIMETYPE_VIDEO_AV1)

/**
 * Whether the platform itself lists a hardware decoder for a type the engine takes, read here from
 * `MediaCodecList` and `isHardwareAccelerated` apart from the SDK's own read, so the engine's answer is
 * compared with the platform's and not with what the SDK handed over.
 */
private fun platformListsAHardwareDecoder(): Boolean =
    MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.any { codec ->
        !codec.isEncoder && codec.isHardwareAccelerated && codec.supportedTypes.any { it in DECODED_TYPES }
    }

/** What `info` says of hardware decoding on [engine]. */
private suspend fun hardwareDecoding(engine: Engine): Boolean = JSONObject(engine.request("info")).getBoolean("hardwareDecoding")

/**
 * What the engine's start does with the list: an engine started through the SDK registers the hardware
 * decoders the device lists, and `info.hardwareDecoding` says so; one started with a list that names none
 * decodes in software, and says that.
 */
internal val hardwareDecodingChecks: List<Case> =
    listOf(
        Case("an engine started through the SDK reports hardware decoding as the device's own list has it") { context ->
            val expected = platformListsAHardwareDecoder()
            // A process keeps the last list it was handed, so an earlier check's list must not stand in for the
            // one this start hands over.
            setDeviceCodecs(emptyList())
            // The start an app calls, the plugin's own, which has to start through the core's to hand the list over.
            val engine = EffectsEngine.start(context, gpu = GpuChoice.CPU_ONLY)
            try {
                val reported = hardwareDecoding(engine.core)
                require(reported == expected) { "info.hardwareDecoding is $reported, but the platform lists a hardware decoder: $expected" }
            } finally {
                engine.shutDown()
            }
        },
        Case("an engine started with a list that names no decoder reports no hardware decoding") { context ->
            setDeviceCodecs(emptyList())
            val engine = Engine.fromBoundary(startEffectsEngine(Folders.appDefault(context).toFfi(), GpuChoice.CPU_ONLY))
            try {
                require(!hardwareDecoding(engine)) { "info.hardwareDecoding is true with no decoder listed" }
            } finally {
                engine.shutDown()
            }
        },
    )
