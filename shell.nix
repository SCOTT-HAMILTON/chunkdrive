{}:
let
  rust-overlay = import (fetchTarball "https://github.com/oxalica/rust-overlay/archive/3eed08a074cd2000884a69d448d70da2843f7103.tar.gz");
  pkgs = import <nixpkgs> {
    overlays = [rust-overlay];
  };
in
with pkgs; mkShell {
  buildInputs = with pkgs; [
    rust-bin.stable.latest.default
    rust-bin.stable.latest.clippy
    cargo-edit
    openssl
    pkg-config
  ];
  RUST_SRC_PATH="${rust-bin.stable.latest.rust-src}/lib/rustlib/src/rust/library/";
}
