{ pkgs ? import <nixpkgs> {} }:

let
  # TODO: Remove once https://github.com/NixOS/nixpkgs/pull/117416 is merged.
  deepspeech = pkgs.stdenv.mkDerivation rec {
    name = "deepspeech-${version}";
    version = "0.9.3";

    src = pkgs.fetchurl {
      url = "https://github.com/mozilla/DeepSpeech/releases/download/v${version}/native_client.amd64.cpu.linux.tar.xz";
      sha256 = "1qy2gspprcxi76jk06ljp028xl0wkk1m3mqaxyf5qbhhfbvvpfap";
    };
    setSourceRoot = "sourceRoot=`pwd`";

    nativeBuildInputs = with pkgs; [
      autoPatchelfHook
    ];

    buildInputs = with pkgs; [
      stdenv.cc.cc.lib
    ];

    installPhase = ''
      install -D deepspeech $out/bin/deepspeech
      install -D deepspeech.h $out/include/deepspeech.h
      install -D libdeepspeech.so $out/lib/libdeepspeech.so
    '';
  };
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    alsaLib
    cargo-edit
    deepspeech
    rustup
  ];

  nativeBuildInputs = with pkgs; [
    pkgconfig
  ];

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang}/lib";
}
