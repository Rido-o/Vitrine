{
  lib,
  rustPlatform,
  dart-sass,
  pkg-config,
  wrapGAppsHook4,
  gdk-pixbuf,
  glycin-loaders,
  gtk4,
  libjpeg_turbo,
  librsvg,
  shared-mime-info,
  webp-pixbuf-loader,
}:
rustPlatform.buildRustPackage {
  pname = "vitrine-rs";
  version = "0.1.0";

  # The crate, plus the TypeScript app's stylesheet (compiled in by build.rs)
  # and icons.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./build.rs
      ./src
      ../src/style.scss
      ../src/theme.scss
      ../icons
    ];
  };
  cargoRoot = "rust";
  buildAndTestSubdir = "rust";
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [dart-sass pkg-config wrapGAppsHook4];
  buildInputs = [gdk-pixbuf gtk4 libjpeg_turbo webp-pixbuf-loader];

  # The TypeScript app's symbolic icons, found through this (main.rs).
  env.VITRINE_ICONS_DIR = "${placeholder "out"}/share/vitrine-rs/icons";

  # As in ../package.nix: our own loaders.cache, so GdkPixbuf reads WebP.
  postInstall = ''
    mkdir -p $out/share/vitrine-rs
    cp -r icons $out/share/vitrine-rs/icons

    cache=$out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache
    mkdir -p $(dirname $cache)
    ${gdk-pixbuf.dev}/bin/gdk-pixbuf-query-loaders \
      ${gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
      ${librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
      ${webp-pixbuf-loader}/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so \
      > $cache
  '';

  dontWrapGApps = true;
  postFixup = ''
    wrapProgram $out/bin/vitrine-rs "''${gappsWrapperArgs[@]}" \
      --set GDK_PIXBUF_MODULE_FILE $out/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache \
      --prefix XDG_DATA_DIRS : ${glycin-loaders}/share \
      --suffix XDG_DATA_DIRS : ${shared-mime-info}/share
  '';

  meta = {
    description = "Spike: a gtk4-rs port of Vitrine";
    mainProgram = "vitrine-rs";
    platforms = lib.platforms.linux;
  };
}
