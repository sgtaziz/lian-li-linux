{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.lian-li-linux;
  # The login PATH, which systemd expands per user.
  userPath = [
    "/run/wrappers"
    "%h/.nix-profile"
    "%h/.local/state/nix/profile"
    "/etc/profiles/per-user/%u"
    "/nix/var/nix/profiles/default"
    "/run/current-system/sw"
  ];
in
{
  options.services.lian-li-linux = {
    enable = lib.mkEnableOption "Lian Li Linux fan, RGB and LCD control";

    package = lib.mkPackageOption pkgs "lian-li-linux" { };

    systemService.enable = lib.mkEnableOption ''
      the system daemon, which runs at boot as the `lianli` account. NixOS keeps
      units read-only, so the mode cannot be selected from the GUI
    '';

    userService.enable = lib.mkEnableOption ''
      the user daemon, which starts at login for every user. NixOS keeps units
      read-only, so the mode cannot be selected from the GUI
    '';

    evdi.enable = lib.mkEnableOption "the EVDI kernel module for desktop-mode LCD panels";
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.systemService.enable && cfg.userService.enable);
        message = "services.lian-li-linux: enable systemService or userService, not both.";
      }
    ];

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
    # System units get a bare PATH. Fonts (fc-match), command sensors (sh) and GPU
    # sensors (nvidia-smi, lspci) need the same tools the user daemon finds.
    systemd.services.lianli-daemon-system.path = [ "/run/current-system/sw" ];

    # Declaring a user unit replaces its login PATH with a bare one, so put it back.
    systemd.user.services = {
      # Other distributions ship this link in the package. NixOS ignores packaged .wants.
      lianli-session = {
        wantedBy = [ "default.target" ];
        path = userPath;
      };
      lianli-daemon = lib.mkIf cfg.userService.enable {
        wantedBy = [ "default.target" ];
        path = userPath;
      };
    };

    boot.extraModulePackages = lib.mkIf cfg.evdi.enable [ config.boot.kernelPackages.evdi ];
    boot.kernelModules = lib.mkIf cfg.evdi.enable [ "evdi" ];
  };
}
