#!/usr/bin/env bash
# Publishes the PKGBUILD staged in /home/builder/pkg to the AUR as <pkgbase>.
# Runs as root in an Arch container with a "builder" user and AUR_SSH_PRIVATE_KEY set.
set -euo pipefail

pkgbase=$1
message=$2
license="$GITHUB_WORKSPACE/packaging/archlinux/LICENSE"
: "${AUR_SSH_PRIVATE_KEY:?AUR_SSH_PRIVATE_KEY is not set}"

cd /home/builder/pkg
chown -R builder .
# Downloads, verifies and unpacks the sources without compiling; a VCS pkgver() also runs here.
sudo -u builder makepkg --nobuild --nodeps --noconfirm
sudo -u builder sh -c 'makepkg --printsrcinfo > .SRCINFO'
grep -E '^\s*(pkgver|source|sha256sums) =' .SRCINFO

# ssh takes its home from passwd, not $HOME, so every path is passed explicitly.
ssh_dir=$(mktemp -d)
(umask 077; printf '%s\n' "$AUR_SSH_PRIVATE_KEY" | tr -d '\r' > "$ssh_dir/aur")
ssh-keyscan -t ed25519 aur.archlinux.org > "$ssh_dir/known_hosts"
# Published on https://aur.archlinux.org; refuse any other host key.
ssh-keygen -lf "$ssh_dir/known_hosts" | grep -q 'SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4'
export GIT_SSH_COMMAND="ssh -i $ssh_dir/aur -o UserKnownHostsFile=$ssh_dir/known_hosts -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes"

git clone "ssh://aur@aur.archlinux.org/$pkgbase.git" /tmp/aur
cp PKGBUILD .SRCINFO lianli-linux.install "$license" /tmp/aur/
cd /tmp/aur
git add PKGBUILD .SRCINFO lianli-linux.install LICENSE
if git diff --cached --quiet; then
  echo "$pkgbase on the AUR is already up to date"
  exit 0
fi
git -c user.name=sgtaziz -c user.email=sgtaziz@users.noreply.github.com commit -m "$message"
git push origin HEAD:master
