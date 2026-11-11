.PHONY: build cli bench test serve clean

build:
	wasm-pack build --target web --out-name lmConnect
	cp pkg/lmConnect.js lib/
	cp pkg/lmConnect_bg.wasm lib/
	cp pkg/lmConnect.d.ts lib/
	cp pkg/lmConnect_bg.wasm.d.ts lib/
	@grep -q 'export function get_memory' lib/lmConnect.js || cat lib/patch.js >> lib/lmConnect.js

cli:
	cargo build --release --features bin --bins

bench:
	node benchmarks/scenarios/prepare.mjs
	cargo build --release --features bin --bin lmc-current

test:
	cargo test
	$(MAKE) build
	node tests/resistance_worker.mjs
	npm ci --prefix example
	npm run build --prefix example

serve: build
	cd example && npm install && npm run dev

clean:
	rm -rf pkg lib/lmConnect.js lib/lmConnect_bg.wasm lib/lmConnect.d.ts lib/lmConnect_bg.wasm.d.ts
