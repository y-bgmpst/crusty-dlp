{
  description = "crusty-dlp";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "crusty-dlp";
          version = "0.6.1";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--bins" ];
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.libGL
            pkgs.wayland
            pkgs.xorg.libX11
            pkgs.xorg.libXcursor
            pkgs.xorg.libXi
            pkgs.xorg.libXrandr
          ];
          postInstall = ''
            install -Dm755 target/release/crusty-dlp $out/bin/crusty-dlp
            install -Dm755 target/release/crusty-dlp-gui $out/bin/crusty-dlp-gui
            install -Dm644 assets/crusty-dlp.desktop $out/share/applications/crusty-dlp.desktop
            install -Dm644 assets/crusty-dlp.svg $out/share/icons/hicolor/scalable/apps/crusty-dlp.svg
            for size in 16 24 32 48 64 128 256 512; do
              install -Dm644 assets/icons/hicolor/${size}x${size}/apps/crusty-dlp.png \
                $out/share/icons/hicolor/${size}x${size}/apps/crusty-dlp.png
            done
            install -Dm644 plugins/yt_dlp_plugins/extractor/boyfriendtv.py \
              $out/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/boyfriendtv.py
            install -Dm644 plugins/yt_dlp_plugins/extractor/ooxxx.py \
              $out/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/ooxxx.py
            install -Dm644 plugins/yt_dlp_plugins/extractor/pmvhaven.py \
              $out/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/pmvhaven.py
            install -Dm644 plugins/yt_dlp_plugins/extractor/spankbang.py \
              $out/share/crusty-dlp/plugins/yt_dlp_plugins/extractor/spankbang.py
          '';
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
          name = "crusty-dlp-gui";
        };
      });
}
