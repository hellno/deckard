{
  description = "Deckard development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;

          commonNativeInputs = with pkgs; [
            rustup
            just
            pkg-config
            nodejs_22
            pnpm
            cargo-deny
          ] ++ lib.optional (pkgs ? foundry) pkgs.foundry;

          linuxBuildInputs = with pkgs; lib.optionals stdenv.isLinux [
            openssl
            gtk3
            libayatana-appindicator
            xdotool
            libxcb
            libxkbcommon
            wayland
            vulkan-headers
            vulkan-loader
            fontconfig
            freetype
          ];

          linuxLibraryPath = lib.optionalString pkgs.stdenv.isLinux (lib.makeLibraryPath (with pkgs; [
            gtk3
            libayatana-appindicator
            xdotool
            libxcb
            libxkbcommon
            wayland
            vulkan-loader
            fontconfig
            freetype
          ]));

          toolPath = lib.makeBinPath commonNativeInputs;
        in
        {
          default = pkgs.mkShell {
            name = "deckard-dev";

            nativeBuildInputs = commonNativeInputs;
            buildInputs = linuxBuildInputs;

            env = {
              DECKARD_NIX_DEV_SHELL = "1";
            } // lib.optionalAttrs pkgs.stdenv.isLinux {
              LD_LIBRARY_PATH = linuxLibraryPath;
            };

            shellHook = ''
              export PATH="${toolPath}:$PATH"

              if ! command -v anvil >/dev/null 2>&1 && [ -d "$HOME/.foundry/bin" ]; then
                export PATH="$HOME/.foundry/bin:$PATH"
              fi

              echo "Deckard dev shell"
              echo "  Rust toolchain: rust-toolchain.toml (rustup)"
              echo "  Checks: cargo fmt --all --check && just check && cargo test --workspace"
              echo "  Browser QA: npm run qa:extension / npm run qa:walletbeat when relevant"
            '';
          };
        });
    };
}
