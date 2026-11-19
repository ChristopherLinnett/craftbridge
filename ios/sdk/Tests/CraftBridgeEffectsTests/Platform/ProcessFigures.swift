// The figures of this process a baseline reads, which only the kernel's task calls give, so their pointers sit here.

import CraftBridge
import Darwin
import Foundation

/// The most memory this process has held since it started, in bytes: the high-water mark of the physical
/// footprint, which is the figure the system's memory-pressure killer holds an app to (`baseline.toml` says why).
func peakFootprint() throws -> Double {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    let status = withUnsafeMutablePointer(to: &info) { pointer in
        pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    guard status == KERN_SUCCESS else {
        throw CraftError.Internal(detail: "task_info(TASK_VM_INFO) answered \(status)")
    }
    return Double(info.ledger_phys_footprint_peak)
}

/// What the exporter names the threads of a pool it sizes below the cores, as the kernel keeps a thread's name.
private let exportPoolThreadName = "craftbridge-export-"

/// The threads the last export ran on, read while its engine lives: the exporter keeps the pool it sized from the
/// free memory until the engine ends, and a job the memory held nothing back for runs on the caller's own threads,
/// one for each processor, which carry no name of the pool's.
func exportThreads() throws -> Double {
    let named = try threadNames().filter { $0.hasPrefix(exportPoolThreadName) }.count
    return Double(named > 0 ? named : ProcessInfo.processInfo.activeProcessorCount)
}

/// The name of every thread of this process, as `thread_info` reads each; a thread that ended between the
/// list and its read is no longer one of them.
private func threadNames() throws -> [String] {
    var list: thread_act_array_t?
    var count: mach_msg_type_number_t = 0
    let listed = task_threads(mach_task_self_, &list, &count)
    guard listed == KERN_SUCCESS, let list else {
        throw CraftError.Internal(detail: "task_threads answered \(listed)")
    }
    defer {
        for index in 0..<Int(count) { mach_port_deallocate(mach_task_self_, list[index]) }
        vm_deallocate(
            mach_task_self_, vm_address_t(UInt(bitPattern: list)), vm_size_t(Int(count) * MemoryLayout<thread_t>.stride)
        )
    }
    return (0..<Int(count)).compactMap { index in
        var info = thread_extended_info_data_t()
        var size = mach_msg_type_number_t(MemoryLayout<thread_extended_info_data_t>.size / MemoryLayout<natural_t>.size)
        let status = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(size)) {
                thread_info(list[index], thread_flavor_t(THREAD_EXTENDED_INFO), $0, &size)
            }
        }
        guard status == KERN_SUCCESS else { return nil }
        return withUnsafeBytes(of: info.pth_name) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
    }
}
