declare module "*.wat" {
  /** The compiled module bytes, produced by the `webassembly-text` Vite plugin. */
  const bytes: Uint8Array<ArrayBuffer>;
  export default bytes;
}
