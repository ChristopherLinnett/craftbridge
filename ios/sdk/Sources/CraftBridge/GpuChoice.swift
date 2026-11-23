// Whether an engine asks for the GPU when it starts. The type is UniFFI's own, declared in the bindings
// module, which is no product: the name here is the one a package outside this one writes it by.

import CraftBridgeFFI

/// What an app asks of the GPU when it starts an engine: `.preferred` takes a device when the phone gives
/// one and renders on the CPU when it gives none, and `.cpuOnly` never asks.
public typealias GpuChoice = CraftBridgeFFI.GpuChoice
