{pkgs, ...}: {
  languages.rust = {
    enable = true;
    channel = "stable";
    components = ["rustc" "cargo" "clippy" "rustfmt" "rust-analyzer"];
  };

  packages = with pkgs; [
    alejandra
    cargo-nextest
    taplo
    treefmt
  ];

  enterShell = ''
    echo "spirit dev env — rust $(rustc --version)"
  '';

  scripts.build.exec = "cargo build --workspace --all-targets";
  scripts."unit-test".exec = "cargo test --workspace";
  scripts.clippy.exec = "cargo clippy --workspace --all-targets -- -D warnings";
  scripts.fmt.exec = "treefmt";
  scripts."fmt-check".exec = "treefmt --fail-on-change";
}
