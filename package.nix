{
  lib,
  rustPlatform,
  makeDesktopItem,
  dart-sass,
  pkg-config,
  wrapGAppsHook4,
  gdk-pixbuf,
  gtk4,
  libjpeg_turbo,
  librsvg,
  shared-mime-info,
  webp-pixbuf-loader,
}: let
  appId = "io.github.Rido_o.Vitrine";
  mimeTypes = ["image/gif" "image/jpeg" "image/png" "image/tiff" "image/webp"];

  desktopItem = makeDesktopItem {
    name = appId;
    desktopName = "Vitrine";
    genericName = "Image Viewer";
    exec = "vitrine %f";
    icon = "image-x-generic";
    categories = ["Graphics" "Viewer"];
    inherit mimeTypes;
    startupWMClass = appId;
  };
in
  rustPlatform.buildRustPackage {
    pname = "vitrine";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./build.rs
        ./src
        ./style
        ./icons
      ];
    };
    cargoLock.lockFile = ./Cargo.lock;

    nativeBuildInputs = [dart-sass pkg-config wrapGAppsHook4];
    buildInputs = [gdk-pixbuf gtk4 libjpeg_turbo webp-pixbuf-loader];

    # The symbolic icons, found through this (main.rs).
    env.VITRINE_ICONS_DIR = "${placeholder "out"}/share/vitrine/icons";

    # Our own loaders.cache, so GdkPixbuf reads WebP (webp-pixbuf-loader isn't
    # in gdk-pixbuf's default cache) and SVG.
    postInstall = ''
      mkdir -p $out/share/vitrine
      cp -r icons $out/share/vitrine/icons
      cp -r ${desktopItem}/share/applications $out/share/

      cache=$out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
      mkdir -p $(dirname $cache)
      ${gdk-pixbuf.dev}/bin/gdk-pixbuf-query-loaders \
        ${gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        ${librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        ${webp-pixbuf-loader}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        > $cache
    '';

    # Wrapped by hand to point GdkPixbuf at that cache (wrapGAppsHook4's
    # wrapper alone would use the default one).
    dontWrapGApps = true;
    postFixup = ''
      wrapProgram $out/bin/vitrine "''${gappsWrapperArgs[@]}" \
        --set GDK_PIXBUF_MODULE_FILE $out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache \
        --suffix XDG_DATA_DIRS : ${shared-mime-info}/share
    '';

    passthru = {inherit appId mimeTypes;};

    meta = {
      description = "A GTK4 image viewer and wallpaper picker";
      mainProgram = "vitrine";
      platforms = lib.platforms.linux;
    };
  }
