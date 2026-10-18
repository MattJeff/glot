// @glot/runtime loads glot.wasm via `new URL("./glot.wasm", import.meta.url)`: keep it out of dep pre-bundling.
export default { optimizeDeps: { exclude: ["@glot/runtime"] } };
