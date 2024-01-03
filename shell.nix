let
  moz_overlay = import (builtins.fetchTarball https://github.com/mozilla/nixpkgs-mozilla/archive/master.tar.gz);
  pkgs = import <nixpkgs> { overlays = [ moz_overlay ]; };
in
pkgs.mkShell {
  buildInputs = [
    pkgs.latest.rustChannels.stable.rust
    pkgs.latest.rustChannels.stable.rust-src
    pkgs.cargo
    pkgs.openssl
    pkgs.pkg-config
  ];
  RUST_SRC_PATH="${pkgs.latest.rustChannels.stable.rust-src}/lib/rustlib/src/rust/library/";
}
