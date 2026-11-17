//! A tiny original WebAssembly effect module for the plug-in tests, in WebAssembly text form —
//! which EffectCraft's loader takes as readily as binary, so the test module is readable here and
//! no fixture file sits in the repository. It is written against the public plug-in ABI (the one
//! documented in EffectCraft's `crates/plugin/src/lib.rs` at the pin): no imports, the memory the
//! manifest sits in, and the exports named, with a renderer that renders nothing. Every test that
//! loads a plug-in asks for its own `id`, because a process registers one plug-in per id.

/// The module bytes — text-form WebAssembly — for a plug-in with this id.
pub(crate) fn module(id: &str) -> Vec<u8> {
    let manifest = format!("{{\"api\":1,\"id\":\"{id}\",\"name\":\"Gate Probe\",\"category\":\"CraftBridge Test\"}}");
    // The manifest's bytes as WebAssembly text escapes, so any id and any wording sit in the
    // segment untouched; the pointer and the length the exports answer are the segment's own.
    // WebAssembly text escapes a byte as a backslash and two hex digits, with no `x` in between.
    let escaped: String = manifest.bytes().map(|byte| format!("\\{byte:02x}")).collect();
    format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 256) "{escaped}")
  (func (export "ec_api_version") (result i32) i32.const 1)
  (func (export "ec_manifest_ptr") (result i32) i32.const 256)
  (func (export "ec_manifest_len") (result i32) i32.const {len})
  (func (export "ec_alloc") (param i32) (result i32) i32.const 65536)
  (func (export "ec_render") (param i32 i32 i32 i32 i32 f64 f64) (result i32) i32.const 0)
)"#,
        len = manifest.len()
    )
    .into_bytes()
}
