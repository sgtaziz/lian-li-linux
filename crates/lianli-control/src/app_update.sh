#!/usr/bin/env bash
set -Eeuo pipefail

boxrun() { /usr/bin/distrobox-enter --name "$box" -- "$@" 8>&- 9>&-; }
valid_lock() { [[ -f $1 && ! -L $1 && $(stat -c %u "$1") == 0 ]]; }
acquire_control_lock() { valid_lock /run/lianli-control.lock && exec 8<>/run/lianli-control.lock && flock -n 8; }
acquire_hardware_lock() { valid_lock /run/lianli-daemon.lock && exec 9<>/run/lianli-daemon.lock && flock -n 9; }
release_hardware_lock() { exec 9>&-; }
release_control_lock() { exec 8>&-; }

service_recovery_help() {
    local unit=$1
    echo 'Service recovery failed. Keep this terminal open and review the error before updating again.' >&2
    echo 'Run these commands in a host terminal to inspect the problem:' >&2
    printf '  systemctl --user status %s --no-pager\n' "$unit" >&2
    printf '  journalctl --user -u %s -b -n 50 --no-pager\n' "$unit" >&2
    echo 'Package verification passed. You can try restarting the service once:' >&2
    printf '  systemctl --user restart %s\n' "$unit" >&2
    echo 'If it still fails, share the output with the project maintainer. Do not repeat the update yet.' >&2
}

restore_services() {
    release_hardware_lock
    if ((daemon_stopped)); then
        if ! boxrun rpm -V lian-li-linux; then
            echo 'Package verification failed. Services remain stopped; repair the package before starting them.' >&2
            echo 'Keep this terminal open and share the output with the project maintainer. Do not retry the update or restart the daemon yet.' >&2
            return 1
        fi
        if ! systemctl --user start lianli-daemon.service ||
            ! systemctl --user is-active --quiet lianli-daemon.service; then
            service_recovery_help lianli-daemon.service
            return 1
        fi
        daemon_stopped=0
    fi
    if ((session_stopped)); then
        if ! systemctl --user start lianli-session.service; then
            service_recovery_help lianli-session.service
            return 1
        fi
        session_stopped=0
    fi
}

finish() {
    local status=$?
    local recovered=1
    trap - EXIT INT TERM HUP
    if ! restore_services; then status=1; recovered=0; fi
    release_control_lock
    if ((status == 0)); then
        echo 'Update completed. Reopening Lian Li Linux.'
        if ! systemd-run --user --collect --property=Type=exec \
            --unit="lianli-app-reopen-$$" -- /usr/bin/distrobox-enter \
            --name "$box" -- /usr/bin/lianli-gui; then
            echo 'Services recovered, but the GUI could not be reopened. Open it from the launcher.' >&2
            status=1
        fi
    else
        echo 'Update did not complete. Review the output above; no success was reported.' >&2
        if ((upgrade_failed && recovered)); then
            echo 'The package update failed, but services were restored. Review the DNF error above, reopen the app, and check for updates to try again.' >&2
        fi
    fi
    rm -f -- "$job/ready" "$job/proceed" "$job/update.sh"
    rmdir -- "$job" 2>/dev/null || true
    read -r -p 'Press Enter to close this terminal. ' _ || true
    exit "$status"
}

main() {
    box=$1
    job=$2
    daemon_stopped=0
    session_stopped=0
    upgrade_failed=0
    [[ $box =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*$ ]] || return 1
    [[ $job == "$HOME"/.local/state/lianli-linux/update-* && -d $job && ! -L $job ]] || return 1
    [[ $(stat -c %u "$job") == "$(id -u)" ]] || return 1
    trap finish EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP
    for tool in flock systemctl sudo pgrep; do command -v "$tool" >/dev/null || return 1; done
    printf 'ready\n' >"$job/ready"
    for ((attempt=0; attempt<40; attempt++)); do
        if [[ -f $job/proceed ]]; then break; fi
        sleep 0.5
    done
    if [[ ! -f $job/proceed ]]; then
        echo 'The app did not confirm the handoff. No services were stopped.' >&2
        return 1
    fi
    echo 'Waiting for the application to close...'
    for ((attempt=0; attempt<120; attempt++)); do
        if ! pgrep -u "$(id -u)" -x lianli-gui >/dev/null; then break; fi
        sleep 0.5
    done
    if pgrep -u "$(id -u)" -x lianli-gui >/dev/null; then
        echo 'The GUI is still open. No services were stopped.' >&2
        return 1
    fi
    acquire_control_lock || { echo 'Another service operation is running.' >&2; return 1; }
    boxrun /usr/bin/lianli-control verify-package-update
    sudo -v
    boxrun sudo -v
    local backup="$HOME/.local/state/lianli-linux/update-backups/$(date +%Y%m%d-%H%M%S)-$$"
    mkdir -p -m 700 "$backup"
    if systemctl --user is-active --quiet lianli-session.service; then
        session_stopped=1
        systemctl --user stop lianli-session.service
    fi
    daemon_stopped=1
    systemctl --user stop lianli-daemon.service
    if systemctl --user is-active --quiet lianli-daemon.service; then
        echo 'The daemon did not stop. Package update cancelled.' >&2
        return 1
    fi
    acquire_hardware_lock || { echo 'Hardware ownership was not released. Package update cancelled.' >&2; return 1; }
    if [[ -d $HOME/.config/lianli ]]; then cp -a "$HOME/.config/lianli" "$backup/config"; fi
    echo "Configuration backup: $backup"
    if ! boxrun sudo dnf upgrade --refresh --setopt=install_weak_deps=False lian-li-linux; then
        upgrade_failed=1
        return 1
    fi
    boxrun cat /usr/lib/udev/rules.d/60-lianli.rules >"$job/60-lianli.rules"
    boxrun cat /usr/lib/tmpfiles.d/lianli.conf >"$job/lianli.conf"
    [[ -s $job/60-lianli.rules && -s $job/lianli.conf ]]
    for path in /etc/udev/rules.d/60-lianli.rules /etc/tmpfiles.d/lianli.conf; do
        if sudo test -f "$path"; then sudo cp -a "$path" "$backup/$(basename "$path")"; fi
    done
    sudo install -m 0644 "$job/60-lianli.rules" /etc/udev/rules.d/60-lianli.rules
    sudo install -m 0644 "$job/lianli.conf" /etc/tmpfiles.d/lianli.conf
    sudo udevadm control --reload-rules
    sudo udevadm trigger
    sudo systemd-tmpfiles --create lianli.conf
    rm -f "$job/60-lianli.rules" "$job/lianli.conf"
    boxrun rpm -q lian-li-linux
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
