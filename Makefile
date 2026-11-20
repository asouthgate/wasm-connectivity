.PHONY: build cli bench test serve clean

WASM_NIGHTLY ?= nightly-2025-11-15
export WASM_THREADS ?= 1

build:
ifeq ($(WASM_THREADS),1)
	RUSTFLAGS="-C target-feature=+atomics,+bulk-memory -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=2147483648 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base" \
		rustup run $(WASM_NIGHTLY) wasm-pack build --target web --out-name lmConnect -- --features wasm-threads -Z build-std=panic_abort,std
else
	wasm-pack build --target web --out-name lmConnect
endif
	cp pkg/lmConnect.js lib/
	cp pkg/lmConnect_bg.wasm lib/
	cp pkg/lmConnect.d.ts lib/
	cp pkg/lmConnect_bg.wasm.d.ts lib/
	@grep -q 'export function get_memory' lib/lmConnect.js || cat lib/patch.js >> lib/lmConnect.js

cli:
	cargo build --release --features bin --bins
	mkdir -p bin
	cp target/release/lmc bin/lmc

bench: cli
	node benchmarks/scenarios/prepare.mjs

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
