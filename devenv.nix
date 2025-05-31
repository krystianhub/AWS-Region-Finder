{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  languages.rust = {
    enable = true;
    channel = "stable";
    targets = [
      "wasm32-unknown-unknown"
    ];
  };

  languages.javascript = {
    enable = true;
    npm.enable = true;
  };

  packages = with pkgs; [
    cargo-generate
  ];

  scripts.build.exec = "cargo build";
  scripts.test.exec = "cargo test";
  scripts.lint.exec = "cargo clippy";
  scripts.format.exec = "cargo fmt";
}
