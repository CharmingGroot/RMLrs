// SPDX-License-Identifier: Apache-2.0
// On macOS an extension module resolves Python symbols at load time.
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
