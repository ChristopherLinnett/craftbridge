// The device's codec list, read from the platform and handed to the engine before it starts.

package io.github.christopherlinnett.craftbridge.core

import android.media.MediaCodecList
import io.github.christopherlinnett.craftbridge.ffi.core.CodecInfo as BoundaryCodecInfo

/**
 * `MediaCodecList(REGULAR_CODECS)` as the boundary takes it, in the list's own order: each codec's name, its
 * types, whether it is an encoder, and the platform's own answer to whether it runs in hardware. The engine
 * picks its hardware decoders from it, since the NDK lists codecs, and says which are hardware, only from
 * API 36, and the platform's default decoder for a type can be a software codec. The regular codecs are the
 * ones the platform's own lookup by type chooses from.
 */
internal fun deviceCodecs(): List<BoundaryCodecInfo> =
    MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.map { codec ->
        BoundaryCodecInfo(
            name = codec.name,
            types = codec.supportedTypes.toList(),
            encoder = codec.isEncoder,
            hardwareAccelerated = codec.isHardwareAccelerated,
        )
    }
