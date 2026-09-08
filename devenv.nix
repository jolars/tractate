{
  pkgs,
  ...
}:

{
  packages = [
    pkgs.go-task
    pkgs.panache
    pkgs.taplo
  ]
  ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.strace ];

  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  git-hooks.hooks = {
    clippy = {
      enable = true;
      settings.allFeatures = true;
    };

    rustfmt.enable = true;

    panache-format = {
      enable = true;
      entry = "panache format --force-exclude";
      files = "\\.(md|qmd|Rmd)$";
      language = "system";
    };
  };
}
