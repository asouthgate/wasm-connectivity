#!/bin/bash
set -e
cd "$(dirname "$0")"

echo "Building WASM..."
wasm-pack build --target web

echo "Copying to web/src/lib/..."
cp pkg/wasm_connect.js web/src/lib/
cp pkg/wasm_connect_bg.wasm web/src/lib/

echo "Patching exports into web/src/lib/wasm_connect.js..."
cat >> web/src/lib/wasm_connect.js << 'PATCH'

export function get_memory() {
    if (!wasmInstance) throw new Error('WASM not initialized');
    return wasmInstance.exports.memory;
}

export function __reset() {
    wasm = undefined;
    wasmInstance = null;
    wasmModule = null;
    cachedDataViewMemory0 = null;
    cachedFloat64ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
}
PATCH

echo "Done."
