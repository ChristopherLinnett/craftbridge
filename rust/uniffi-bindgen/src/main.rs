//! The binding generator for every `-ffi` library in the workspace, run in library mode.
//!
//! UniFFI's generator must be the version of the `uniffi` the libraries are built with, since it
//! reads the metadata they embed. Building it here from the workspace's one `uniffi` pin is what
//! keeps the two equal, as UniFFI's own documentation sets it up.

fn main() {
    uniffi::uniffi_bindgen_main();
}
