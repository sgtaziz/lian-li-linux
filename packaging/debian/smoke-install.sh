#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" != 0 ] || [ -d /run/systemd/system ] || [ "$#" != 1 ]; then
    echo "Run only in a disposable root container without systemd, with one package." >&2
    exit 1
fi
if [ ! -f /.dockerenv ] && [ ! -f /run/.containerenv ]; then
    echo "Refusing package smoke testing outside a disposable container." >&2
    exit 1
fi
package=$(realpath "$1")
[ "$(dpkg-deb -f "$package" Package)" = lian-li-linux ]
version=$(dpkg-deb -f "$package" Version)
previous_version="${version}~upgrade-test"
dpkg --compare-versions "$previous_version" lt "$version"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
dpkg-deb --raw-extract "$package" "$stage/previous"
awk -v version="$previous_version" '$1 == "Version:" { print "Version: " version; next } { print }' \
    "$stage/previous/DEBIAN/control" > "$stage/control"
mv "$stage/control" "$stage/previous/DEBIAN/control"
dpkg-deb --build --root-owner-group "$stage/previous" "$stage/previous.deb"
apt-get install -y --no-install-recommends "$stage/previous.deb"
getent passwd lianli
[ -f /run/lianli-daemon.lock ]
[ "$(stat -c %a /run/lianli-daemon.lock)" = 666 ]
[ -f /run/lianli-control.lock ]
[ "$(stat -c %a /run/lianli-control.lock)" = 666 ]
[ "$(stat -c %U:%a /var/lib/lianli-control)" = root:700 ]
for binary in /usr/bin/lianli-daemon /usr/bin/lianli-gui /usr/bin/lianli-session /usr/bin/lianli-control; do
    libraries=$(ldd "$binary")
    if grep -q 'not found' <<< "$libraries"; then
        echo "Missing loader dependency for $binary" >&2
        exit 1
    fi
done
if find /etc/systemd /usr/lib/systemd -type l -path '*.wants/lianli-daemon*.service' -print -quit | grep -q .; then
    echo "Installation unexpectedly enabled a hardware service." >&2
    exit 1
fi
mkdir -p /var/lib/lianli
printf 'preserve user configuration\n' > /var/lib/lianli/package-smoke-sentinel
printf '{"hardware_video":true,"release_smoke":"preserve"}\n' > /var/lib/lianli/config.json
install -d /var/lib/lianli/profiles
printf '{"hardware_video":false,"release_smoke":"profile"}\n' > /var/lib/lianli/profiles/smoke.json
chown lianli:lianli /var/lib/lianli/config.json /var/lib/lianli/profiles/smoke.json
chmod 600 /var/lib/lianli/config.json /var/lib/lianli/profiles/smoke.json
install -d -m755 /etc/lianli
printf '{"version":1,"selection":{"scope":"system","uid":%s}}\n' "$(id -u lianli)" > /etc/lianli/service-selection.json
chmod 644 /etc/lianli/service-selection.json
manifest="$stage/retained-state"
sha256sum /var/lib/lianli/package-smoke-sentinel /var/lib/lianli/config.json \
    /var/lib/lianli/profiles/smoke.json /etc/lianli/service-selection.json > "$manifest"
lock_identity=$(stat -c '%d:%i:%u:%g:%a' /run/lianli-daemon.lock /run/lianli-control.lock)
state_identity=$(stat -c '%d:%i:%u:%g:%a' /var/lib/lianli/config.json /var/lib/lianli/profiles/smoke.json)
verify_retained_state() {
    sha256sum --check "$manifest"
    [ "$(stat -c '%d:%i:%u:%g:%a' /run/lianli-daemon.lock /run/lianli-control.lock)" = "$lock_identity" ]
    [ "$(stat -c '%d:%i:%u:%g:%a' /var/lib/lianli/config.json /var/lib/lianli/profiles/smoke.json)" = "$state_identity" ]
    [ "$(stat -c '%u:%g:%a' /etc/lianli/service-selection.json)" = '0:0:644' ]
    if find /etc/systemd /usr/lib/systemd -type l -path '*.wants/lianli-daemon*.service' -print -quit | grep -q .; then
        echo "Package lifecycle unexpectedly enabled a hardware service." >&2
        exit 1
    fi
}
apt-get install -y --no-install-recommends "$package"
[ "$(dpkg-query -W -f='${Version}' lian-li-linux)" = "$version" ]
verify_retained_state
printf 'Synthetic older-version package upgrade preserved configuration, profiles, selection and lock identity.\n'
apt-get install -y --reinstall --no-install-recommends "$package"
verify_retained_state
apt-get remove -y lian-li-linux
verify_retained_state
apt-get purge -y lian-li-linux
verify_retained_state
apt-get install -y --no-install-recommends "$package"
verify_retained_state
if find /etc/systemd /usr/lib/systemd -type l -path '*.wants/lianli-daemon*.service' -print -quit | grep -q .; then
    echo "Reinstallation unexpectedly enabled a hardware service." >&2
    exit 1
fi
apt-get purge -y lian-li-linux
verify_retained_state
