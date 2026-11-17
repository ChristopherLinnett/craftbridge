(module
  (memory (export "memory") 1)
  (data (i32.const 256) "{\"api\":1,\"id\":\"io.github.christopherlinnett.craftbridge.conformance\",\"name\":\"Gate Probe\",\"category\":\"CraftBridge Test\"}")
  (func (export "ec_api_version") (result i32) i32.const 1)
  (func (export "ec_manifest_ptr") (result i32) i32.const 256)
  (func (export "ec_manifest_len") (result i32) i32.const 119)
  (func (export "ec_alloc") (param i32) (result i32) i32.const 65536)
  (func (export "ec_render") (param i32 i32 i32 i32 i32 f64 f64) (result i32) i32.const 0)
)
