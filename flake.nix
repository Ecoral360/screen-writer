{
  description = "screen-writer: draw on your screen (gromit-mpx inspired)";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-25.11";

  outputs =
    { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.screen-writer;

        screen-writer = pkgs.rustPlatform.buildRustPackage {
          pname = "screen-writer";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ];
          };
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.makeWrapper ];

          # Runtime tools: fc-match (font lookup) and gsettings (GNOME shortcut installer).
          postInstall = ''
            wrapProgram $out/bin/screen-writer \
              --suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.fontconfig.bin pkgs.glib.bin ]}
          '';

          meta = {
            description = "Draw on your screen with lots of tools (F9 to toggle, Shift+F9 to clear)";
            mainProgram = "screen-writer";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      overlays.default = final: _prev: {
        screen-writer = self.packages.${final.stdenv.hostPlatform.system}.default;
      };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}
