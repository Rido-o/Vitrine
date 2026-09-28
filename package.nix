{
  lib,
  stdenv,
  runtimeShell,
  makeDesktopItem,
  dart-sass,
  esbuild,
  gobject-introspection,
  wrapGAppsHook4,
  gdk-pixbuf,
  gexiv2_0_16,
  gjs,
  glib,
  gtk4,
  librsvg,
  webp-pixbuf-loader,
  gnim,
}: let
  appId = "io.github.Rido_o.Vitrine";

  desktopItem = makeDesktopItem {
    name = appId;
    desktopName = "Vitrine";
    genericName = "Image Viewer";
    exec = "vitrine %f";
    icon = "image-x-generic";
    categories = ["Graphics" "Viewer"];
    mimeTypes = ["image/jpeg" "image/png" "image/webp"];
    startupWMClass = appId;
  };
in
  stdenv.mkDerivation {
    pname = "vitrine";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [./src ./icons];
    };

    nativeBuildInputs = [
      dart-sass
      esbuild
      gobject-introspection
      wrapGAppsHook4
    ];

    buildInputs = [
      gdk-pixbuf
      gexiv2_0_16
      gjs
      glib
      gtk4
      webp-pixbuf-loader
    ];

    buildPhase = ''
      runHook preBuild

      # gnim's package.json exports ./dist, which its build copies from ./src
      mkdir -p node_modules
      cp -r ${gnim} node_modules/gnim
      chmod -R u+w node_modules/gnim
      ln -s src node_modules/gnim/dist

      sass --no-source-map src/style.scss src/style.css

      esbuild src/main.tsx \
        --bundle --format=esm --outfile=main.js \
        --external:'gi://*' --external:'resource://*' \
        --external:system --external:gettext --external:cairo \
        --jsx=automatic --jsx-import-source=gnim/gtk4 \
        --loader:.css=text \
        --define:ICONS_DIR="'$out/share/vitrine/icons'"

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin $out/share/vitrine
      cp main.js $out/share/vitrine/
      cp -r icons $out/share/vitrine/
      cp -r ${desktopItem}/share/applications $out/share/

      # wrapGAppsHook points GdkPixbuf at librsvg's loaders.cache, which has
      # no WebP; build one with the webp loader too (used for thumbnails).
      cache=$out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
      mkdir -p $(dirname $cache)
      ${gdk-pixbuf.dev}/bin/gdk-pixbuf-query-loaders \
        ${gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        ${librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        ${webp-pixbuf-loader}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
        > $cache

      cat > $out/bin/vitrine <<EOF
      #!${runtimeShell}
      exec ${gjs}/bin/gjs -m $out/share/vitrine/main.js "\$@"
      EOF
      chmod +x $out/bin/vitrine

      runHook postInstall
    '';

    # Wrap by hand so our loaders.cache comes after (and overrides) the hook's
    # own GDK_PIXBUF_MODULE_FILE.
    dontWrapGApps = true;
    postFixup = ''
      wrapProgram $out/bin/vitrine "''${gappsWrapperArgs[@]}" \
        --set GDK_PIXBUF_MODULE_FILE $out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
    '';

    passthru = {inherit appId;};

    meta = {
      description = "GTK4 image viewer and wallpaper picker";
      homepage = "https://github.com/Rido-o/Vitrine";
      mainProgram = "vitrine";
      platforms = lib.platforms.linux;
    };
  }
