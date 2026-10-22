{
  description = "nekomata — a tabbed, split-pane GUI for felis";

  inputs = {
    felis.url = "git+https://git.natsukium.com/felis-terminal/felis?rev=71f4a5ca11ed9bee3422444c52fc425e88e00a81";
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
              postInstall = ''
                install -Dm644 ${./share/fish/vendor_conf.d/nekomata.fish} \
                  $out/share/fish/vendor_conf.d/nekomata.fish
              '';
              postFixup = ''
                wrapProgram $out/bin/nekomata \
                  --prefix PATH : "$out/bin:${pkgs.lib.makeBinPath [ felis.packages.${system}.felis ]}" \
                  --set-default NEKOMATA_FONT "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf" \
                  --set-default XDG_DATA_DIRS /usr/local/share:/usr/share \
                  --prefix XDG_DATA_DIRS : "$out/share" \
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
              fish
              rust-analyzer
              felis.packages.${system}.felis
            ];
            NEKOMATA_FONT = "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf";
            nativeBuildInputs = [ pkgs.pkg-config ];
            shellHook = ''
              export XDG_DATA_DIRS="${./share}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
            '';
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
      checks = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          build = self.packages.${system}.default;
          fish-integration =
            pkgs.runCommand "nekomata-fish-integration"
              {
                nativeBuildInputs = [
                  pkgs.bash
                  pkgs.fish
                ];
              }
              ''
                cp -r ${./share} ./share
                bash ${./scripts/test-fish-integration.sh}
                touch $out
              '';
        }
      );
      formatter = forAllSystems (system: (import nixpkgs { inherit system; }).nixfmt);
    };
}
