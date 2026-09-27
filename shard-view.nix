{inputs, ...}: {
  perSystem = {pkgs, ...}: let
    desktopItem = pkgs.makeDesktopItem {
      name = "dev.shard.View";
      desktopName = "shard-view";
      genericName = "Image Viewer";
      exec = "shard-view %f";
      icon = "image-x-generic";
      categories = ["Graphics" "Viewer"];
      mimeTypes = ["image/jpeg" "image/png" "image/webp"];
      startupWMClass = "dev.shard.View";
    };
  in {
    packages.shard-view = pkgs.stdenv.mkDerivation {
      pname = "shard-view";
      version = "0.1.0";
      src = ./.;

      nativeBuildInputs = with pkgs; [
        dart-sass
        esbuild
        gobject-introspection
        wrapGAppsHook4
      ];

      buildInputs = with pkgs; [
        gdk-pixbuf
        gjs
        glib
        gtk4
        webp-pixbuf-loader
      ];

      buildPhase = ''
        runHook preBuild

        # gnim's package.json exports ./dist, which its build copies from ./src
        mkdir -p node_modules
        cp -r ${inputs.gnim} node_modules/gnim
        chmod -R u+w node_modules/gnim
        ln -s src node_modules/gnim/dist

        cp ${../ags-shell/config/theme.scss} src/theme.scss
        sass --no-source-map src/style.scss src/style.css

        esbuild src/main.tsx \
          --bundle --format=esm --outfile=main.js \
          --external:'gi://*' --external:'resource://*' \
          --external:system --external:gettext --external:cairo \
          --jsx=automatic --jsx-import-source=gnim/gtk4 \
          --loader:.css=text \
          --define:ICONS_DIR="'$out/share/shard-view/icons'"

        runHook postBuild
      '';

      installPhase = ''
        runHook preInstall

        mkdir -p $out/bin $out/share/shard-view
        cp main.js $out/share/shard-view/
        cp -r icons $out/share/shard-view/
        cp -r ${desktopItem}/share/applications $out/share/

        # wrapGAppsHook points GdkPixbuf at librsvg's loaders.cache, which has
        # no WebP; build one with the webp loader too (used for thumbnails).
        cache=$out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
        mkdir -p $(dirname $cache)
        ${pkgs.gdk-pixbuf.dev}/bin/gdk-pixbuf-query-loaders \
          ${pkgs.gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
          ${pkgs.librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
          ${pkgs.webp-pixbuf-loader}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
          > $cache

        cat > $out/bin/shard-view <<EOF
        #!${pkgs.runtimeShell}
        exec ${pkgs.gjs}/bin/gjs -m $out/share/shard-view/main.js "\$@"
        EOF
        chmod +x $out/bin/shard-view

        runHook postInstall
      '';

      # Wrap by hand so our loaders.cache comes after (and overrides) the
      # hook's own GDK_PIXBUF_MODULE_FILE.
      dontWrapGApps = true;
      postFixup = ''
        wrapProgram $out/bin/shard-view "''${gappsWrapperArgs[@]}" \
          --set GDK_PIXBUF_MODULE_FILE $out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
      '';
    };
  };

  flake.modules.homeManager.shard-view = {
    config,
    lib,
    pkgs,
    ...
  }: let
    cfg = config.shard.shard-view;
    base = inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.shard-view;
  in {
    options.shard.shard-view.wallpaperCommand = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "set-wallpaper";
      description = "Command run with an image path to set it as the wallpaper; null hides the action.";
    };

    options.shard.shard-view.defaultViewer = lib.mkEnableOption ''
      making shard-view the default for the image types it handles. Set with
      xdg-mime on each activation, so ~/.config/mimeapps.list stays unmanaged
      (other defaults and Thunar's "Open With" keep working)
    '';

    config.home.activation.shard-view-default = lib.mkIf cfg.defaultViewer (
      lib.hm.dag.entryAfter ["writeBoundary"] ''
        run ${pkgs.xdg-utils}/bin/xdg-mime default dev.shard.View.desktop \
          image/jpeg image/png image/webp
      ''
    );

    config.home.packages = [
      (
        if cfg.wallpaperCommand == null
        then base
        else
          pkgs.symlinkJoin {
            name = "shard-view";
            paths = [base];
            nativeBuildInputs = [pkgs.makeBinaryWrapper];
            postBuild = ''
              wrapProgram $out/bin/shard-view \
                --set-default SHARD_VIEW_WALLPAPER_COMMAND ${lib.escapeShellArg cfg.wallpaperCommand}
            '';
          }
      )
    ];
  };
}
