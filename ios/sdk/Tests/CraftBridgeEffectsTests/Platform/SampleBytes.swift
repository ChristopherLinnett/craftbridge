// The tests' read of a sample buffer's data, which Core Media owns, as plain bytes.

import CoreMedia

/// The bytes of `buffer`'s data, or none when it holds none or Core Media cannot copy it.
func bytes(of buffer: CMSampleBuffer) -> [UInt8] {
    guard let block = CMSampleBufferGetDataBuffer(buffer) else { return [] }
    var data = [UInt8](repeating: 0, count: CMBlockBufferGetDataLength(block))
    let copied = data.withUnsafeMutableBytes { bytes -> Bool in
        guard let base = bytes.baseAddress else { return true }
        return CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: bytes.count, destination: base)
            == kCMBlockBufferNoErr
    }
    return copied ? data : []
}
