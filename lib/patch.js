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
