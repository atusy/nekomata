{
  description = "nekomata — a tabbed, split-pane GUI for felis";

  inputs = {
    felis.url = "git+https://git.natsukium.com/felis-terminal/felis?rev=2a3030b237947497708b962ba6be8c2adade4951";
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
            fontconfig
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
              # Rendering regression tests must use the same font as the app.
              NEKOMATA_FONT = "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf";
              postInstall = ''
                install -Dm644 ${./share/fish/vendor_conf.d/nekomata.fish} \
                  $out/share/fish/vendor_conf.d/nekomata.fish
              '';
              postFixup = ''
                executable="$out/bin/nekomata"
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  app="$out/Applications/nekomata.app/Contents"
                  mkdir -p "$app/MacOS"
                  cp ${./share/macos/Info.plist} "$app/Info.plist"
                  mv "$executable" "$app/MacOS/nekomata-bin"
                  executable="$app/MacOS/nekomata-bin"
                ''}
                wrapperArgs=(
                  --prefix PATH : "$out/bin:${
                    pkgs.lib.makeBinPath (
                      [ felis.packages.${system}.felis ]
                      ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.openssh ]
                    )
                  }" \
                  --set-default NEKOMATA_FONT "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf" \
                  --set-default XDG_DATA_DIRS /usr/local/share:/usr/share \
                  --prefix XDG_DATA_DIRS : "$out/share" \
                  ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "--prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}"}
                )
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  substitute ${./share/macos/launcher.c} launcher.c \
                    --subst-var-by package "$out" \
                    --subst-var-by felis "${felis.packages.${system}.felis}" \
                    --subst-var-by font "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf"
                  $CC -Wall -Wextra -Werror launcher.c -o "$app/MacOS/nekomata"
                ''}
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
                  mv "$executable" "$out/bin/nekomata-bin"
                  executable="$out/bin/nekomata-bin"
                ''}
                makeWrapper "$executable" "$out/bin/nekomata" "''${wrapperArgs[@]}"
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  /usr/bin/codesign --force --sign - --identifier io.github.atusy.nekomata.bin \
                    "$app/MacOS/nekomata-bin"
                  /usr/bin/codesign --force --sign - --identifier io.github.atusy.nekomata \
                    "$out/Applications/nekomata.app"
                  /usr/bin/codesign --verify --deep --strict "$out/Applications/nekomata.app"
                ''}
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
            packages =
              with pkgs;
              [
                cargo
                rustc
                rustfmt
                clippy
                fish
                rust-analyzer
                felis.packages.${system}.felis
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.openssh ];
            NEKOMATA_FONT = "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf";
            nativeBuildInputs = [ pkgs.pkg-config ];
            shellHook = ''
              export XDG_DATA_DIRS="${./share}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
            '';
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
              with pkgs;
              [
                fontconfig
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
                  fontconfig
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
