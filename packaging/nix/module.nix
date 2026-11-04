{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.lian-li-linux;
in
{
  options.services.lian-li-linux = {
    enable = lib.mkEnableOption "Lian Li Linux fan, RGB and LCD control";

    package = lib.mkPackageOption pkgs "lian-li-linux" { };

    serviceMode = lib.mkOption {
      type = lib.types.nullOr (
        lib.types.enum [
          "user"
          "system"
        ]
      );
      default = null;
      description = ''
        Daemon to start automatically. `user` runs it in the graphical session,
        `system` runs it at boot as the `lianli` account. `null` leaves both
        stopped so the mode can be chosen from the GUI.
      '';
    };

    evdi.enable = lib.mkEnableOption "the EVDI kernel module for desktop-mode LCD panels";
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    services.udev.packages = [ cfg.package ];
    systemd.packages = [ cfg.package ];
    systemd.tmpfiles.packages = [ cfg.package ];

    security.polkit.enable = true;
    environment.etc."polkit-1/rules.d/49-lianli-recovery.rules".source =
      "${cfg.package}/share/polkit-1/rules.d/49-lianli-recovery.rules";

    users.users.lianli = {
      isSystemUser = true;
      group = "lianli";
      description = "Lian Li Linux daemon";
    };
    users.groups.lianli = { };

    systemd.services.lianli-control-recovery.wantedBy = [ "multi-user.target" ];
    systemd.services.lianli-daemon-system.wantedBy = lib.mkIf (cfg.serviceMode == "system") [
      "multi-user.target"
    ];
    systemd.user.services.lianli-daemon.wantedBy = lib.mkIf (cfg.serviceMode == "user") [
      "default.target"
    ];
    systemd.user.services.lianli-session.wantedBy = [ "default.target" ];

    boot.extraModulePackages = lib.mkIf cfg.evdi.enable [ config.boot.kernelPackages.evdi ];
    boot.kernelModules = lib.mkIf cfg.evdi.enable [ "evdi" ];
  };
}
