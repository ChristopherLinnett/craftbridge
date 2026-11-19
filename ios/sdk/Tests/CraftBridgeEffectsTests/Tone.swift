// The sound the tests that export a movie with sound share: a second of silence and then a tone, so where the
// tone starts in what a reader gives says, to the sample, whether the sound is where the pictures are.

import Foundation

/// `value` as `count` little-endian bytes.
func littleEndian(_ value: Int, bytes count: Int) -> [UInt8] {
    (0..<count).map { UInt8(truncatingIfNeeded: value >> (8 * $0)) }
}

/// The sound's rate, in samples a second.
let rate = 48_000

/// How long the sound is silent before its tone begins, in seconds.
let silentSeconds = 1

/// The samples of that silence.
let silence = silentSeconds * rate

/// The sample at which the sound's tone first rises past ``loud``: its first ``silentSeconds`` are silent, and the tone
/// rises from zero, so its sixth sample is the first past it.
let onset = silence + 5

/// A level no sample of the silence, or of the AAC encoder's ringing before the tone, reaches.
let loud = 3_000

/// A 48 kHz 16-bit stereo WAV of `seconds`: ``silentSeconds`` of silence, then a 440 Hz tone from zero, loud
/// enough to tell from silence.
func tone(seconds: Int) -> Data {
    var samples: [UInt8] = []
    samples.reserveCapacity(seconds * rate * 4)
    for index in 0..<(seconds * rate) {
        let level = index < silence ? 0 : Int(sin(Double(index - silence) * 440 * 2 * .pi / Double(rate)) * 12_000)
        samples += littleEndian(level, bytes: 2) + littleEndian(level, bytes: 2)
    }
    let header: [UInt8] =
        Array("RIFF".utf8) + littleEndian(36 + samples.count, bytes: 4) + Array("WAVEfmt ".utf8)
        + littleEndian(16, bytes: 4) + littleEndian(1, bytes: 2) + littleEndian(2, bytes: 2)
        + littleEndian(rate, bytes: 4) + littleEndian(rate * 4, bytes: 4) + littleEndian(4, bytes: 2)
        + littleEndian(16, bytes: 2) + Array("data".utf8) + littleEndian(samples.count, bytes: 4)
    return Data(header + samples)
}
