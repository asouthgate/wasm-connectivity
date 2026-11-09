.PHONY: build bench test serve clean

build:
	wasm-pack build --target web
	cp pkg/wasm_connect.js lib/
	cp pkg/wasm_connect_bg.wasm lib/
	cp pkg/wasm_connect.d.ts lib/
	cp pkg/wasm_connect_bg.wasm.d.ts lib/
	@grep -q 'export function get_memory' lib/wasm_connect.js || cat lib/patch.js >> lib/wasm_connect.js

bench:
	node benchmarks/scenarios/prepare.mjs
	cargo build --release --features bin --bin scenario-solve

test:
	cargo test
	$(MAKE) build
	node tests/resistance_worker.mjs
	npm ci --prefix example
	npm run build --prefix example

serve: build
	cd example && npm install && npm run dev

clean:
	rm -rf pkg lib/wasm_connect.js lib/wasm_connect_bg.wasm lib/wasm_connect.d.ts lib/wasm_connect_bg.wasm.d.ts
