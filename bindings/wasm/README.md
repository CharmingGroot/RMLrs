# rmlrs (WebAssembly)

A thin wrapper over the RMLrs library for browsers and Node. There is no
file system, so every source the mapping reads is passed in memory, keyed
by its `rml:path`. Output matches the native CLI byte for byte on all 76
RML-Core cases (`tests/core_parity.mjs`).

```js
import init, { materialize } from "./pkg/rmlrs_wasm.js";
await init();
const nquads = materialize(mappingTtl, { "data.json": jsonText }, "http://example.com/");
```

## Build

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
cargo build --release -p rmlrs-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/rmlrs_wasm.wasm
```

Use `--target nodejs` for Node. The `.wasm` file is about 430 KB.
