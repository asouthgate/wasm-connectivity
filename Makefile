.PHONY: build serve clean

build:
	wasm-pack build --target web
	cp pkg/wasm_connect.js lib/
	cp pkg/wasm_connect_bg.wasm lib/
	cp pkg/wasm_connect.d.ts lib/
	cp pkg/wasm_connect_bg.wasm.d.ts lib/
	@grep -q 'export function get_memory' lib/wasm_connect.js || cat lib/patch.js >> lib/wasm_connect.js

serve: build
	cd example && npm install && npm run dev

clean:
	rm -rf pkg lib/wasm_connect.js lib/wasm_connect_bg.wasm lib/wasm_connect.d.ts lib/wasm_connect_bg.wasm.d.ts
