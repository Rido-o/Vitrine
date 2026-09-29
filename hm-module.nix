self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.vitrine;
  wrapped =
    if cfg.wallpaperCommand == null
    then cfg.package
    else
      pkgs.symlinkJoin {
        name = "vitrine";
        paths = [cfg.package];
        nativeBuildInputs = [pkgs.makeBinaryWrapper];
        postBuild = ''
          wrapProgram $out/bin/${cfg.package.meta.mainProgram} \
            --set-default VITRINE_WALLPAPER_COMMAND ${lib.escapeShellArg cfg.wallpaperCommand}
        '';
      };
in {
  options.programs.vitrine = {
    enable = lib.mkEnableOption "Vitrine, a GTK4 image viewer and wallpaper picker";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "vitrine.packages.\${system}.default";
      description = "The Vitrine package.";
    };

    wallpaperCommand = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "swww img";
      description = ''
        Command run with an image path appended to set it as the wallpaper
        (sets VITRINE_WALLPAPER_COMMAND). Null hides the "Set wallpaper"
        action.
      '';
    };

    defaultViewer = lib.mkEnableOption ''
      making Vitrine the default for the image types it handles. It's set with
      xdg-mime on each activation, so ~/.config/mimeapps.list stays unmanaged
      and other defaults keep working
    '';
  };

  config = lib.mkIf cfg.enable {
    home.packages = [wrapped];

    home.activation.vitrine-default-viewer = lib.mkIf cfg.defaultViewer (
      lib.hm.dag.entryAfter ["writeBoundary"] ''
        run ${pkgs.xdg-utils}/bin/xdg-mime default ${cfg.package.appId}.desktop \
          ${lib.concatStringsSep " " cfg.package.mimeTypes}
      ''
    );
  };
}
