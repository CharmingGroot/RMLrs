// SPDX-License-Identifier: Apache-2.0
// Runs every RML-Core case through the WebAssembly build (all files of the
// case passed in memory, keyed by name) and compares with the native CLI.
//
//   node core_parity.mjs <pkg dir from wasm-bindgen --target nodejs> <rmlrs binary> <suites dir>
import { readFileSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";

const [pkg, binaryArg, suites] = process.argv.slice(2);
const binary = path.resolve(binaryArg);
const wasm = createRequire(import.meta.url)(path.resolve(pkg, "rmlrs_wasm.js"));
const cases = path.resolve(suites, "rml-core", "test-cases");
const rows = readFileSync(path.join(cases, "metadata.csv"), "utf8").trim().split("\n").slice(1);
let same = 0, differ = 0;
for (const row of rows) {
  const id = row.split(",")[0];
  const dir = path.join(cases, id);
  const base = row.match(/,(https?:\/\/[^,]*),mapping\.ttl,/)?.[1] ?? "http://example.com/";
  const sources = Object.fromEntries(readdirSync(dir).filter((f) => !f.endsWith(".ttl") && !f.endsWith(".md")).map((f) => [f, readFileSync(path.join(dir, f), "utf8")]));
  let native = null, js = null;
  try { native = execFileSync(binary, ["run", path.join(dir, "mapping.ttl"), "--base-iri", base], { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); } catch {}
  try { js = wasm.materialize(readFileSync(path.join(dir, "mapping.ttl"), "utf8"), sources, base); } catch {}
  if (native === js) same++; else { differ++; console.log("differs:", id); }
}
console.log(`same ${same} / ${same + differ}`);
process.exit(differ ? 1 : 0);
