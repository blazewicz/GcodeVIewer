;; A module whose only function returns a `v128`, the value type that only
;; WebAssembly SIMD engines can validate. The bundle builds it with the
;; `webassembly-text` plugin in vite.config.ts; renderer.ts validates it to
;; detect engines that cannot run the SIMD build of the wasm module.

(module
  (func (export "simd") (result v128)
    v128.const i32x4 0 0 0 0))
