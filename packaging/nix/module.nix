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

    systemService.enable = lib.mkEnableOption ''
      the system daemon, which runs at boot as the `lianli` account. NixOS keeps
      system units read-only, so this mode cannot be selected from the GUI. Leave
      it disabled to choose the user daemon from the GUI instead
    '';

    evdi.enable = lib.mkEnableOption "the EVDI kernel module for desktop-mode LCD panels";
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    services.udev.packages = [ cfg.package ];
    systemd.packages = [ cfg.package ];
    systemd.tmpfiles.packages = [ cfg.package ];

    security.polkit.enable = true;
    security.polkit.enablePkexecWrapper = true;
    environment.etc."polkit-1/rules.d/49-lianli-recovery.rules".source =
      "${cfg.package}/share/polkit-1/rules.d/49-lianli-recovery.rules";

    users.users.lianli = {
      isSystemUser = true;
      group = "lianli";
      description = "Lian Li Linux daemon";
    };
    users.groups.lianli = { };

    systemd.services.lianli-control-recovery.wantedBy = [ "multi-user.target" ];
    systemd.services.lianli-daemon-system.wantedBy = lib.mkIf cfg.systemService.enable [
      "multi-user.target"
    ];

    boot.extraModulePackages = lib.mkIf cfg.evdi.enable [ config.boot.kernelPackages.evdi ];
    boot.kernelModules = lib.mkIf cfg.evdi.enable [ "evdi" ];
  };
}
