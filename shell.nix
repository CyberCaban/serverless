{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  name = "rust web dev";

  nativeBuildInputs = with pkgs; [
    pkg-config
    openssl
    jq
  ];

  buildInputs = with pkgs; [
    # System dependencies
    openssl.dev
    pkg-config
  ];

  # Environment variables
  env = {
    # Required for openssl-sys
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
  };

  # Shell hook for convenience
  shellHook = ''
    alias cr='cargo run'
    alias crr='cargo run --release'
    echo "🚀 Rust web development environment ready!"
  '';
}
