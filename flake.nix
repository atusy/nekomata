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
      # Lets tests dlopen fontconfig as the app does at runtime.
      fontconfigLibraryPath =
        pkgs:
        let
          variable =
            if pkgs.stdenv.hostPlatform.isDarwin then "DYLD_FALLBACK_LIBRARY_PATH" else "LD_LIBRARY_PATH";
        in
        ''
          export ${variable}="${
            pkgs.lib.makeLibraryPath [ pkgs.fontconfig ]
          }''${${variable}:+:''$${variable}}"
        '';
      # The pinned font set felis's dev shell names in FELIS_TEST_FONT_DIR
      # (felis dev/flake-module.nix testFonts); its renderer tests require it.
      felisTestFonts =
        pkgs:
        pkgs.runCommand "felis-test-fonts" { nativeBuildInputs = [ pkgs.python3Packages.fonttools ]; } ''
          mkdir -p $out/share/fonts/opentype $out/share/fonts/truetype
          for family in Neon Argon ArgonNF Xenon; do
            for face in ${pkgs.monaspace}/share/fonts/opentype/Monaspace$family-*.otf; do
              case "$face" in
                *Wide*) ;;
                *) cp "$face" $out/share/fonts/opentype/ ;;
              esac
            done
          done
          cp ${pkgs.monaspace}/share/fonts/truetype/MonaspaceNeonFrozen-{Regular,Bold,Italic,BoldItalic,WideRegular}.ttf \
            "${pkgs.monaspace}/share/fonts/truetype/Monaspace Neon Var.ttf" \
            $out/share/fonts/truetype/
          cp ${pkgs.noto-fonts}/share/fonts/noto/NotoSansSymbols.ttf $out/share/fonts/
          cp ${pkgs.noto-fonts}/share/fonts/noto/NotoSansSymbols2-Regular.otf $out/share/fonts/
          cp ${pkgs.noto-fonts-color-emoji}/share/fonts/noto/NotoColorEmoji.ttf $out/share/fonts/
          pyftsubset ${pkgs.noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc \
            --font-number=0 --unicodes=U+845B,U+E0100-E0101 --output-file=$out/share/fonts/NotoSansCJKjp-Kuzu.otf
          pyftsubset ${pkgs.noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc \
            --font-number=2 --unicodes=U+845B,U+E0100-E0101 --output-file=$out/share/fonts/NotoSansCJKsc-Kuzu.otf
        '';
      # Crane inputs shared by the package and the Cargo checks, so they reuse
      # one dependency build.
      rustBuild =
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          craneLib = crane.mkLib pkgs;
          runtimeLibs = with pkgs; [
            fontconfig
            vulkan-loader
            libGL
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
          ];
          common = {
            # The embedded renderer compiles its WGSL shaders with include_str!.
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter =
                path: type:
                craneLib.filterCargoSources path type
                || pkgs.lib.hasSuffix ".wgsl" path
                || pkgs.lib.hasSuffix ".png" path
                || builtins.baseNameOf path == "README.md"
                || builtins.baseNameOf path == "LICENSE";
            };
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
          inherit
            craneLib
            runtimeLibs
            common
            cargoArtifacts
            ;
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          inherit (rustBuild system)
            craneLib
            runtimeLibs
            common
            cargoArtifacts
            ;
          fontconfigPath = fontconfigLibraryPath pkgs;
        in
        {
          default = craneLib.buildPackage (
            common
            // {
              inherit cargoArtifacts;
              # Legacy rendering fixtures use an explicit font independent of user config.
              NEKOMATA_FONT = "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf";
              # Font lookup tests load fontconfig at runtime; fail rather than skip.
              NEKOMATA_REQUIRE_FONTCONFIG = "1";
              preCheck = fontconfigPath;
              postInstall = ''
                install -Dm644 ${./share/icons/nekomata.png} \
                  $out/share/icons/hicolor/256x256/apps/nekomata.png
                install -Dm644 ${./share/icons/nekomata.svg} \
                  $out/share/icons/hicolor/scalable/apps/nekomata.svg
                install -Dm644 ${./share/fish/vendor_conf.d/nekomata.fish} \
                  $out/share/fish/vendor_conf.d/nekomata.fish
              '';
              postFixup = ''
                executable="$out/bin/nekomata"
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  app="$out/Applications/nekomata.app/Contents"
                  mkdir -p "$app/MacOS" "$app/Resources"
                  cp ${./share/icons/nekomata.icns} "$app/Resources/nekomata.icns"
                  cp ${./share/macos/Info.plist} "$app/Info.plist"
                  mv "$executable" "$app/MacOS/nekomata-bin"
                  executable="$app/MacOS/nekomata-bin"
                ''}
                wrapperArgs=(
                  --prefix PATH : "$out/bin:${
                    pkgs.lib.makeBinPath (
                      [ felis.packages.${system}.felis ]
                      ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                        pkgs.glib.bin
                        pkgs.openssh
                        pkgs.xdg-utils
                      ]
                    )
                  }" \
                  --set-default XDG_DATA_DIRS /usr/local/share:/usr/share \
                  --prefix XDG_DATA_DIRS : "$out/share" \
                  ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux "--prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}"}
                )
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
                  substitute ${./share/macos/launcher.c} launcher.c \
                    --subst-var-by package "$out" \
                    --subst-var-by felis "${felis.packages.${system}.felis}"
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
                resvg
                python3
                jq
                clippy
                fish
                rust-analyzer
                felis.packages.${system}.felis
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                pkgs.glib.bin
                pkgs.openssh
                pkgs.xdg-utils
              ];
            NEKOMATA_FONT = "${pkgs.udev-gothic-nf}/share/fonts/truetype/UDEVGothicNF-Regular.ttf";
            nativeBuildInputs = [ pkgs.pkg-config ];
            NEKOMATA_REQUIRE_FONTCONFIG = "1";
            shellHook = ''
              export XDG_DATA_DIRS="${./share}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
            ''
            + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin (fontconfigLibraryPath pkgs);
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
              with pkgs;
              [
                fontconfig
                vulkan-loader
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
                  vulkan-loader
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
          inherit (rustBuild system) craneLib common cargoArtifacts;
        in
        {
          build = self.packages.${system}.default;
          clippy = craneLib.cargoClippy (
            common
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";
              doInstallCargoArtifacts = false;
            }
          );
          fmt = craneLib.cargoFmt { inherit (common) src; };
          # nekomata's embedding patch lives in the vendored renderer, whose
          # upstream tests read the pinned fonts and the golden fixtures that
          # are not vendored; both come from the pinned felis source.
          felis-render-wgpu = craneLib.cargoTest (
            common
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs =
                "-p felis-render-wgpu --lib"
                # Upstream pins its golden instance buffers on Linux only; on
                # macOS they diverge even in the upstream checkout.
                + pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isDarwin " -- --skip row_cache::golden_tests";
              FELIS_TEST_FONT_DIR = "${felisTestFonts pkgs}/share/fonts";
              preCheck = ''
                mkdir -p vendor/felis-render-wgpu/tests
                cp -r ${felis}/crates/felis-render-wgpu/tests/fixtures vendor/felis-render-wgpu/tests/
                chmod -R u+w vendor/felis-render-wgpu/tests
              '';
              doInstallCargoArtifacts = false;
            }
          );
          felis-revision =
            pkgs.runCommand "nekomata-felis-revision"
              {
                nativeBuildInputs = [
                  pkgs.bash
                  pkgs.jq
                  pkgs.python3
                ];
              }
              ''
                bash ${./.}/scripts/test-felis-revision.sh
                bash ${./.}/scripts/check-felis-revision.sh
                touch $out
              '';
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
