#!/usr/bin/env bash
set -Eeuo pipefail
workflow=$(cd "$(dirname "$0")/../src" && pwd)/app_update.sh
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

for scenario in success upgrade_failure verification_failure lock_failure rpm_failure restart_failure handoff_failure; do
    home="$temporary/$scenario"
    mkdir -p "$home/.local/state/lianli-linux/update-test"
    touch "$home/.local/state/lianli-linux/update-test/proceed"
    if [[ $scenario == handoff_failure ]]; then rm "$home/.local/state/lianli-linux/update-test/proceed"; fi
    export HOME="$home" LOG="$home/log" SCENARIO="$scenario" WORKFLOW="$workflow"
    set +e
    bash <<'MOCK' >"$home/output" 2>&1
source "$WORKFLOW"
boxrun() {
    echo "box $*" >>"$LOG"
    case "$*" in
        '/usr/bin/lianli-control verify-package-update') [[ $SCENARIO != verification_failure ]] ;;
        'sudo dnf '*) [[ $SCENARIO != upgrade_failure ]] ;;
        'rpm -V '*) [[ $SCENARIO != rpm_failure ]] ;;
        'cat '*) printf 'mock rule\n' ;;
        *) return 0 ;;
    esac
}
acquire_control_lock() { echo control-lock >>"$LOG"; }
acquire_hardware_lock() { echo hardware-lock >>"$LOG"; [[ $SCENARIO != lock_failure ]]; }
release_hardware_lock() { echo hardware-release >>"$LOG"; }
release_control_lock() { echo control-release >>"$LOG"; }
pgrep() { return 1; }
sleep() { return 0; }
sudo() { echo "host sudo $*" >>"$LOG"; [[ $1 != test ]]; }
systemctl() {
    echo "service $*" >>"$LOG"
    case "$*" in
        '--user stop lianli-daemon.service') touch "$HOME/stopped" ;;
        '--user start lianli-daemon.service') [[ $SCENARIO != restart_failure ]] && rm "$HOME/stopped" ;;
        '--user is-active --quiet lianli-daemon.service') [[ ! -f $HOME/stopped ]] ;;
        *) return 0 ;;
    esac
}
systemd-run() { echo reopen >>"$LOG"; }
main lianli "$HOME/.local/state/lianli-linux/update-test"
MOCK
    status=$?
    set -e
    if [[ $scenario == success ]]; then
        [[ $status == 0 ]]
        grep -q '^reopen$' "$LOG"
        stop=$(grep -n 'service --user stop lianli-daemon.service' "$LOG" | cut -d: -f1)
        upgrade=$(grep -n 'box sudo dnf ' "$LOG" | cut -d: -f1)
        start=$(grep -n 'service --user start lianli-daemon.service' "$LOG" | cut -d: -f1)
        ((stop < upgrade && upgrade < start))
    else
        [[ $status != 0 ]]
        ! grep -q '^reopen$' "$LOG"
        if [[ $scenario == verification_failure || $scenario == handoff_failure ]]; then
            ! grep -q 'service --user stop' "$LOG"
        elif [[ $scenario == rpm_failure ]]; then
            grep -q 'Do not retry the update or restart the daemon yet' "$home/output"
            ! grep -q 'You can try restarting the service once' "$home/output"
            ! grep -q 'service --user start' "$LOG"
            [[ -f $HOME/stopped ]]
        elif [[ $scenario == restart_failure ]]; then
            grep -q 'systemctl --user status lianli-daemon.service --no-pager' "$home/output"
            grep -q 'journalctl --user -u lianli-daemon.service -b -n 50 --no-pager' "$home/output"
            grep -q 'systemctl --user restart lianli-daemon.service' "$home/output"
            ! grep -q 'but services were restored' "$home/output"
            grep -q 'service --user start lianli-daemon.service' "$LOG"
            [[ -f $HOME/stopped ]]
        else
            if [[ $scenario == upgrade_failure ]]; then
                grep -q 'package update failed, but services were restored' "$home/output"
            fi
            grep -q 'service --user start lianli-daemon.service' "$LOG"
            [[ ! -f $HOME/stopped ]]
        fi
        if [[ $scenario == lock_failure ]]; then ! grep -q 'box sudo dnf ' "$LOG"; fi
    fi
    echo "PASS: $scenario"
done
