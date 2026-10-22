{
  description = "nekomata — a tabbed, split-pane GUI for felis";

  inputs = {
    felis.url = "git+https://git.natsukium.com/natsukium/felis?rev=1761d2a035d6ccf26da004f395e38b93da2f5686";
    nixpkgs.follows = "felis/nixpkgs";
    crane.follows = "felis/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      felis,
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          craneLib = crane.mkLib pkgs;
          runtimeLibs = with pkgs; [
            libGL
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
          ];
          common = {
            src = craneLib.cleanCargoSource ./.;
            strictDeps = true;
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.makeWrapper
            ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux runtimeLibs;
          };
          cargoArtifacts = craneLib.buildDepsOnly common;
        in
        {
          default = craneLib.buildPackage (
            common
            // {
              inherit cargoArtifacts;
              postFixup = ''
                wrapProgram $out/bin/nekomata \
                  --prefix PATH : ${pkgs.lib.makeBinPath [ felis.packages.${system}.felis ]} \
                  ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "--prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}"}
              '';
            }
          );
        }
      );
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              felis.packages.${system}.felis
            ];
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
              with pkgs;
              [
                libGL
                libxkbcommon
                wayland
                libx11
                libxcursor
                libxi
                libxrandr
              ]
            );
            LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
              pkgs.lib.makeLibraryPath (
                with pkgs;
                [
                  libGL
                  libxkbcommon
                  wayland
                  libx11
                  libxcursor
                  libxi
                  libxrandr
                ]
              )
            );
          };
        }
      );
      checks = forAllSystems (system: {
        build = self.packages.${system}.default;
      });
      formatter = forAllSystems (system: (import nixpkgs { inherit system; }).nixfmt);
    };
}
