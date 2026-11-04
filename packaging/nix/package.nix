{
  lib,
  rustPlatform,
  src,
  fetchNpmDeps,
  npmHooks,
  nodejs,
  pkg-config,
  cmake,
  nasm,
  wrapGAppsHook3,
  addDriverRunpath,
  libusb1,
  udev,
  ffmpeg,
  fontconfig,
  freetype,
  libGL,
  libgbm,
  libxkbcommon,
  wayland,
  libx11,
  libinput,
  libdrm,
  libjpeg_turbo,
  dbus,
  openssl,
  webkitgtk_4_1,
  gtk3,
  glib,
  libsoup_3,
  librsvg,
  libayatana-appindicator,
  linuxPackages,
  systemd,
  runtimeShell,
  coreutils,
}:

let
  cargoToml = lib.importTOML "${src}/Cargo.toml";
  frontendRoot = "crates/lianli-gui";
in
rustPlatform.buildRustPackage {
  pname = "lian-li-linux";
  version = cargoToml.workspace.package.version or cargoToml.package.version;

  inherit src;

  cargoLock.lockFile = "${src}/Cargo.lock";

  npmRoot = frontendRoot;
  npmDeps = fetchNpmDeps {
    name = "lianli-gui-npm-deps";
    src = "${src}/${frontendRoot}";
    hash = "sha256-fyxkk7BTX5/VAi9BAHgc3MIb3q+JDLBt0Q/lBNMlkMs=";
  };

  nativeBuildInputs = [
    pkg-config
    cmake
    nasm
    nodejs
    npmHooks.npmConfigHook
    rustPlatform.bindgenHook
    wrapGAppsHook3
    addDriverRunpath
  ];

  buildInputs = [
    libusb1
    udev
    ffmpeg
    fontconfig
    freetype
    libGL
    libgbm
    libxkbcommon
    wayland
    libx11
    libinput
    libdrm
    libjpeg_turbo
    dbus
    openssl
    webkitgtk_4_1
    gtk3
    glib
    libsoup_3
    librsvg
    libayatana-appindicator
  ];

  postPatch = ''
    files=$(grep -rlE '/usr/bin/(systemctl|systemd-run|busctl|pkexec|lianli-(control|daemon|session|gui))' crates/*/src \
      | grep -vE 'distrobox|container' || true)
    for file in $files; do
      substituteInPlace "$file" \
        --replace-quiet '/usr/bin/systemctl' '${systemd}/bin/systemctl' \
        --replace-quiet '/usr/bin/systemd-run' '${systemd}/bin/systemd-run' \
        --replace-quiet '/usr/bin/busctl' '${systemd}/bin/busctl' \
        --replace-quiet '/usr/bin/pkexec' '/run/wrappers/bin/pkexec' \
        --replace-quiet '/usr/bin/lianli-' "$out/bin/lianli-"
    done
  '';

  preBuild = ''
    pushd ${frontendRoot}
    npm run build
    popd
  '';

  doCheck = false;

  dontWrapGApps = true;

  postInstall = ''
    find $out/bin -type f ! -name 'lianli-*' -delete
    rm -f $out/lib/liblianli_gui_lib.*

    install -Dm644 packaging/udev/60-lianli.rules -t $out/lib/udev/rules.d
    substituteInPlace $out/lib/udev/rules.d/60-lianli.rules \
      --replace-fail "/bin/sh -c 'exec chmod" "${runtimeShell} -c 'exec ${coreutils}/bin/chmod"

    for unit in packaging/systemd/*.service; do
      case "$unit" in
        *-system.service|*-recovery.service) dir=$out/lib/systemd/system ;;
        *) dir=$out/lib/systemd/user ;;
      esac
      install -Dm644 "$unit" -t "$dir"
    done
    substituteInPlace $out/lib/systemd/*/*.service --replace-quiet /usr/bin/ $out/bin/

    install -Dm644 packaging/polkit/49-lianli-recovery.rules -t $out/share/polkit-1/rules.d
    install -Dm644 packaging/sysusers.d/lianli.conf $out/lib/sysusers.d/lianli.conf
    install -Dm644 packaging/tmpfiles.d/lianli.conf $out/lib/tmpfiles.d/lianli.conf

    install -Dm644 packaging/desktop/com.sgtaziz.lianlilinux.desktop -t $out/share/applications
    install -Dm644 packaging/desktop/com.sgtaziz.lianlilinux.metainfo.xml -t $out/share/metainfo
    install -Dm644 packaging/desktop/com.sgtaziz.lianlilinux.recovery.desktop -t $out/etc/xdg/autostart
    install -Dm644 packaging/desktop/com.sgtaziz.lianlilinux.session.desktop -t $out/etc/xdg/autostart
    substituteInPlace $out/etc/xdg/autostart/*.desktop --replace-quiet /usr/bin/ $out/bin/

    install -Dm644 assets/icons/32x32.png $out/share/icons/hicolor/32x32/apps/com.sgtaziz.lianlilinux.png
    install -Dm644 assets/icons/128x128.png $out/share/icons/hicolor/128x128/apps/com.sgtaziz.lianlilinux.png
    install -Dm644 assets/icons/128x128@2x.png $out/share/icons/hicolor/256x256/apps/com.sgtaziz.lianlilinux.png
    install -Dm644 assets/icons/icon.svg $out/share/icons/hicolor/scalable/apps/com.sgtaziz.lianlilinux.svg
    install -Dm644 packaging/man/*.1 -t $out/share/man/man1
  '';

  postFixup = ''
    for bin in $out/bin/*; do
      patchelf --add-rpath ${
        lib.makeLibraryPath [
          libGL
          libgbm
          wayland
          libxkbcommon
          linuxPackages.evdi
        ]
      } "$bin"
      addDriverRunpath "$bin"
    done
    wrapProgram $out/bin/lianli-daemon --prefix PATH : ${lib.makeBinPath [ ffmpeg ]}
    wrapProgram $out/bin/lianli-control --prefix PATH : ${lib.makeBinPath [ ffmpeg ]}
    wrapGApp $out/bin/lianli-gui --prefix PATH : ${lib.makeBinPath [ ffmpeg ]}
  '';

  meta = {
    description = "Fan, RGB and LCD control for Lian Li hardware";
    homepage = "https://github.com/sgtaziz/lian-li-linux";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "lianli-gui";
  };
}
