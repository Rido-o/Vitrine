{inputs, ...}: {
  perSystem = {pkgs, ...}: {
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

  flake.modules.homeManager.shard-view = {pkgs, ...}: {
    home.packages = [inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.shard-view];
  };
}
