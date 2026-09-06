Name:           lian-li-linux
Version:        0.8.11
Release:        1%{?dist}
Summary:        Open-source Linux replacement for L-Connect 3

%global evdi_version 1.15.0

License:        MIT
URL:            https://github.com/sgtaziz/lian-li-linux
Source0:        %{name}-%{version}.tar.gz
Source1:        https://github.com/DisplayLink/evdi/archive/refs/tags/v%{evdi_version}.tar.gz#/evdi-%{evdi_version}.tar.gz

%global debug_package %{nil}

BuildRequires:  make gcc
BuildRequires:  cargo
BuildRequires:  pkg-config
BuildRequires:  clang
BuildRequires:  cmake
BuildRequires:  nasm
BuildRequires:  systemd-rpm-macros

BuildRequires:  pkgconfig(libusb-1.0)
# libdrm is needed directly by the bundled libevdi build.
BuildRequires:  pkgconfig(libdrm)
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(appindicator3-0.1)
# Fedora ships ffmpeg-free; full ffmpeg from rpmfusion is needed for H.264 LCD.
BuildRequires:  pkgconfig(libavcodec)
BuildRequires:  pkgconfig(libavformat)
BuildRequires:  pkgconfig(libswscale)
BuildRequires:  pkgconfig(libavutil)

Requires:       hicolor-icon-theme
Requires:       ffmpeg
# evdi kernel module for desktop-mode devices
Recommends:     displaylink

Requires(post): systemd
Requires(preun): systemd
Requires(postun): systemd

%description
Open-source Linux replacement for L-Connect 3: fan speed control, RGB/LED
effects, LCD streaming, and sensor gauges for Lian Li devices.

%prep
%setup -q -a 1

%build
%make_build -C evdi-%{evdi_version}/library
export LIBRARY_PATH="$PWD/evdi-%{evdi_version}/library${LIBRARY_PATH:+:$LIBRARY_PATH}"
export CARGO_PROFILE_RELEASE_STRIP=symbols
cargo build --release --frozen

%install
install -Dpm755 evdi-%{evdi_version}/library/libevdi.so.%{evdi_version} \
    %{buildroot}%{_libdir}/libevdi.so.%{evdi_version}
ln -s libevdi.so.%{evdi_version} %{buildroot}%{_libdir}/libevdi.so.1

install -Dpm755 target/release/lianli-daemon %{buildroot}%{_bindir}/lianli-daemon
install -Dpm755 target/release/lianli-gui     %{buildroot}%{_bindir}/lianli-gui
install -Dpm644 packaging/udev/60-lianli.rules %{buildroot}%{_udevrulesdir}/60-lianli.rules
install -Dpm644 packaging/systemd/lianli-daemon.service %{buildroot}%{_userunitdir}/lianli-daemon.service
install -Dpm644 packaging/systemd/lianli-daemon-system.service %{buildroot}%{_unitdir}/lianli-daemon-system.service
install -Dpm644 packaging/tmpfiles.d/lianli.conf %{buildroot}%{_tmpfilesdir}/lianli.conf
install -Dpm644 packaging/modules-load.d/lianli-evdi.conf %{buildroot}%{_modulesloaddir}/lianli-evdi.conf
install -Dpm644 packaging/desktop/com.sgtaziz.lianlilinux.desktop %{buildroot}%{_datadir}/applications/com.sgtaziz.lianlilinux.desktop
install -Dpm644 assets/icons/32x32.png      %{buildroot}%{_datadir}/icons/hicolor/32x32/apps/com.sgtaziz.lianlilinux.png
install -Dpm644 assets/icons/128x128.png    %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/com.sgtaziz.lianlilinux.png
install -Dpm644 assets/icons/128x128@2x.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/com.sgtaziz.lianlilinux.png
install -Dpm644 assets/icons/icon.svg       %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.sgtaziz.lianlilinux.svg

%pre
getent group lianli >/dev/null || groupadd -r lianli
getent passwd lianli >/dev/null || \
    useradd -r -g lianli -d / -s /sbin/nologin -c "Lian Li daemon" lianli

%post
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger >/dev/null 2>&1 || :
[ -e /sys/module/evdi ] && udevadm trigger --action=add /sys/module/evdi >/dev/null 2>&1 || :
systemd-tmpfiles --create lianli.conf >/dev/null 2>&1 || :
systemctl --system daemon-reload >/dev/null 2>&1 || :
for d in /run/user/*; do
    [ -d "$d" ] && [ -S "$d/bus" ] || continue
    uid="${d##*/}"
    user="$(id -nu "$uid" 2>/dev/null)" || continue
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$d" DBUS_SESSION_BUS_ADDRESS="unix:path=$d/bus" \
        systemctl --user daemon-reload >/dev/null 2>&1 || :
done

%preun
if [ "$1" -eq 0 ]; then
    systemctl --global disable lianli-daemon.service >/dev/null 2>&1 || :
    systemctl --system disable --now lianli-daemon-system.service >/dev/null 2>&1 || :
fi

%postun
if [ "$1" -ge 1 ]; then
    systemctl --system daemon-reload >/dev/null 2>&1 || :
    systemctl --system try-restart lianli-daemon-system.service >/dev/null 2>&1 || :
    for d in /run/user/*; do
        [ -d "$d" ] && [ -S "$d/bus" ] || continue
        uid="${d##*/}"
        user="$(id -nu "$uid" 2>/dev/null)" || continue
        runuser -u "$user" -- env XDG_RUNTIME_DIR="$d" DBUS_SESSION_BUS_ADDRESS="unix:path=$d/bus" \
            systemctl --user try-restart lianli-daemon.service >/dev/null 2>&1 || :
    done
fi

%files
%license LICENSE
%{_bindir}/lianli-daemon
%{_bindir}/lianli-gui
%{_udevrulesdir}/60-lianli.rules
%{_userunitdir}/lianli-daemon.service
%{_unitdir}/lianli-daemon-system.service
%{_tmpfilesdir}/lianli.conf
%{_modulesloaddir}/lianli-evdi.conf
%{_datadir}/applications/com.sgtaziz.lianlilinux.desktop
%{_datadir}/icons/hicolor/32x32/apps/com.sgtaziz.lianlilinux.png
%{_datadir}/icons/hicolor/128x128/apps/com.sgtaziz.lianlilinux.png
%{_datadir}/icons/hicolor/256x256/apps/com.sgtaziz.lianlilinux.png
%{_datadir}/icons/hicolor/scalable/apps/com.sgtaziz.lianlilinux.svg

%package evdi
Summary:        Bundled libevdi library for %{name}
Provides:       libevdi.so.1()(64bit)

%description evdi
Bundled libevdi (v%{evdi_version}) userspace library, used by lianli-daemon.

%files evdi
%{_libdir}/libevdi.so.1
%{_libdir}/libevdi.so.%{evdi_version}

%changelog
* Fri Aug 07 2026 sgtaziz <sgtaziz013@gmail.com> - 0.7.6-1
- Initial Fedora / COPR build.
