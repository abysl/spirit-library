let
  pkgs = import (builtins.fetchTarball "https://github.com/NixOS/nixpkgs/archive/c8f90650c15282fa8656a041bfbbd2403997a9a7.tar.gz") {};
in
  pkgs.mkShellNoCC {
    packages = with pkgs; [treefmt rustfmt taplo alejandra];
  }
