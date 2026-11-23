// What an engine's start made of the files an export left behind when the system killed the app. The types
// are UniFFI's own, declared in the bindings module, which is no product: the names here are the ones a
// package outside this one writes them by.

import CraftBridgeFFI

/// What the start did about the pending-export journal it found: the files and folders it deleted, the ones
/// it left untouched, the ones it refused to touch and the ones it could not delete, each of those two with
/// why, and what it found of the journal files themselves. Every path is text.
public typealias Recovery = CraftBridgeFFI.Recovery

/// One path a start did not delete, and why: an entry of a ``Recovery``'s `refused` and `failed`.
public typealias RecoveryNote = CraftBridgeFFI.RecoveryNote
