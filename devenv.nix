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

  # These four are the contract with CI: .woodpecker/spirit.yml runs exactly
  # these scripts, so a check that passes locally passes there for the same
  # reason. Change them here, not in the pipeline.
  #
  # NOTE: the test script is `unit-test`, not `test`. A script named `test` is
  # unreachable: bash resolves `test` to its builtin (and then to coreutils)
  # long before any devenv script, so `devenv shell -- test` silently runs
  # `[ ]` and exits 1 without ever invoking cargo. ozai's devenv.nix has this
  # latent bug today.
  scripts.build.exec = "cargo build --workspace --all-targets";
  scripts."unit-test".exec = "cargo test --workspace";
  scripts.clippy.exec = "cargo clippy --workspace --all-targets -- -D warnings";
  # Formatting is repo-wide on purpose: one root treefmt.toml covers every
  # project, so a change spanning several of them lands in one consistent style.
  # These two format the whole monorepo, not just spirit.
  scripts.fmt.exec = "treefmt";
  scripts."fmt-check".exec = "treefmt --fail-on-change";
}
