#!/usr/bin/env node
// Verifies that the wasm-pack binary in ui/pkg uses SIMD arithmetic. The
// workspace builds wasm32 with `-C target-feature=+simd128` (see
// .cargo/config.toml), which the renderer's `Vec3A` camera math turns into
// `f32x4` instructions. The flag alone proves little: SIMD memory operations
// and the UTF-8 scanning in the wasm-bindgen stack would pass a looser search
// while the camera stayed scalar. `just ui-build` runs this after the
// wasm-pack build, so a build that loses its vector math fails instead of
// shipping.

import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import wabtFactory from "wabt";

const uiDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const pkgDir = path.join(uiDir, "pkg");

// The instructions glam emits for `Vec3A` multiplication and addition.
const REQUIRED = ["f32x4.mul", "f32x4.add"];

// The binary to inspect: a path argument, or the wasm-pack output.
function findWasm() {
  if (process.argv[2] !== undefined) {
    return path.resolve(process.argv[2]);
  }

  let candidates;
  try {
    candidates = readdirSync(pkgDir).filter((name) => name.endsWith("_bg.wasm"));
  } catch {
    console.error(`Missing ${path.relative(process.cwd(), pkgDir)}; run \`just wasm\` first.`);
    process.exit(1);
  }

  if (candidates.length !== 1) {
    console.error(`expected one *_bg.wasm in ${path.relative(process.cwd(), pkgDir)}, found ${candidates.length}`);
    process.exit(1);
  }
  return path.join(pkgDir, candidates[0]);
}

const wasmPath = findWasm();
const name = path.relative(process.cwd(), wasmPath);

let disassembly;
try {
  const wabt = await wabtFactory();
  const module = wabt.readWasm(readFileSync(wasmPath), { readDebugNames: false, simd: true });
  disassembly = module.toText({});
  module.destroy();
} catch (error) {
  console.error(`cannot disassemble ${name}: ${error instanceof Error ? error.message : error}`);
  process.exit(1);
}

const counts = new Map();
for (const match of disassembly.matchAll(/\b(?:i8x16|i16x8|i32x4|i64x2|f32x4|f64x2|v128)\.\w+/g)) {
  counts.set(match[0], (counts.get(match[0]) ?? 0) + 1);
}

const missing = REQUIRED.filter((opcode) => !counts.has(opcode));
if (missing.length > 0) {
  console.error(`SIMD check failed: ${name} contains no ${missing.join(" or ")} instruction.`);
  console.error("The wasm build is not using vector math; check the +simd128 flag in .cargo/config.toml and the Vec3A camera math in crates/renderer/src/camera.rs.");
  process.exit(1);
}

function count(predicate) {
  return [...counts].filter(([opcode]) => predicate(opcode)).reduce((sum, [, n]) => sum + n, 0);
}

const total = count(() => true);
const float = count((opcode) => opcode.startsWith("f32x4.") || opcode.startsWith("f64x2."));
const detail = REQUIRED.map((opcode) => `${opcode} x${counts.get(opcode)}`).join(", ");
console.log(`SIMD check passed: ${name} has ${total} SIMD instructions (${float} float), including ${detail}`);
