#!/bin/sh
# Netem exists only on this fixture's veth pair; host/system qdiscs are untouched.
set -eu
action=$1
fixture=$2
delay=${3:-10}
bind=${4:-127.0.0.1}
send_buffer=${5:-0}
receive_buffer=${6:-0}
case "$receive_buffer" in 0|4194304) ;; *) exit 2 ;; esac
token=${fixture#/var/tmp/capstan-m1-ssh-}
[ "$fixture" = "/var/tmp/capstan-m1-ssh-$token" ]
[ "${#token}" -eq 12 ]
case "$token" in *[!0-9a-f]*) exit 2 ;; esac
case "$delay" in *[!0-9]*|'') exit 2 ;; esac
[ "$delay" -le 100 ]
case "$send_buffer" in *[!0-9]*|'') exit 2 ;; esac
[ "$send_buffer" -eq 0 ] || { [ "$send_buffer" -ge 65536 ] && [ "$send_buffer" -le 4194304 ]; }
case "$bind" in *[!0-9.]*|'') exit 2 ;; esac
if [ "$bind" != 127.0.0.1 ]; then
    ip -o -4 address show | awk '{split($4,a,"/"); print a[1]}' | grep -Fxq "$bind"
fi
[ "$receive_buffer" -eq 0 ] || { [ "$delay" -gt 0 ] && [ "$send_buffer" -eq 4194304 ]; }
namespace="cm2$token"
short=$(printf '%s' "$token" | cut -c 1-8)
host="cm2h$short"
peer="cm2p$short"
owned="/var/tmp/capstan-m2-net-$token"
script=$(readlink -f "$0")
proxy="$(dirname "$script")/m2-net-proxy.py"

cleanup() {
    [ -d "$owned" ] || return 0
    [ "$(cat "$owned/owner")" = "$token" ]
    relay_failure=0
    if [ -f "$owned/proxy.pid" ]; then
        pid=$(cat "$owned/proxy.pid")
        case "$pid" in *[!0-9]*|'') exit 2 ;; esac
        if [ -r "/proc/$pid/cmdline" ] && tr '\000' '\n' < "/proc/$pid/cmdline" | grep -Fxq "$proxy"; then
            # New relays also carry the owner in argv. Preserve cleanup for a
            # legacy fixture started before this script was updated.
            if [ -f "$owned/proxy-owner" ]; then
                [ "$(cat "$owned/proxy-owner")" = "$token" ]
                tr '\000' '\n' < "/proc/$pid/cmdline" | grep -Fxq "$token"
            fi
            kill "$pid" 2>/dev/null || true
            attempts=0
            while [ "$attempts" -lt 50 ] && [ -r "/proc/$pid/cmdline" ] && tr '\000' '\n' < "/proc/$pid/cmdline" 2>/dev/null | grep -Fxq "$proxy"; do
                sleep 0.1
                attempts=$((attempts + 1))
            done
            if [ -r "/proc/$pid/cmdline" ] && tr '\000' '\n' < "/proc/$pid/cmdline" 2>/dev/null | grep -Fxq "$proxy"; then
                # The retained PID still has this exact fixture identity.
                if [ -f "$owned/proxy-owner" ]; then
                    tr '\000' '\n' < "/proc/$pid/cmdline" | grep -Fxq "$token"
                fi
                kill -KILL "$pid" 2>/dev/null || true
                relay_failure=1
            fi
        fi
    fi
    if [ -f "$owned/proxy.stdout" ]; then
        # Only fixed numeric relay records, never data or arbitrary stderr.
        awk '/^M2_RELAY / {print}' "$owned/proxy.stdout"
        if [ -f "$owned/proxy-owner" ]; then
            if ! python3 -c 'import json, sys
records = [json.loads(line[len("M2_RELAY "):]) for line in open(sys.argv[1]) if line.startswith("M2_RELAY ")]
if len(records) != 1:
    raise SystemExit(1)
record = records[0]
if (record["activeClients"] != 0 or record["ownedWriters"] != 0
        or record["socketConfigurationFailures"] != 0 or not 0 <= record["peakClients"] <= 16):
    raise SystemExit(1)
' "$owned/proxy.stdout"; then relay_failure=1; fi
        fi
    elif [ -f "$owned/proxy-owner" ]; then
        relay_failure=1
    fi
    if ip netns list | awk '{print $1}' | grep -Fxq "$namespace"; then
        # Only processes inside this owned, freshly created network namespace.
        for pid in $(ip netns pids "$namespace"); do kill "$pid" 2>/dev/null || true; done
        ip netns del "$namespace"
    fi
    if ip link show "$host" >/dev/null 2>&1; then ip link del "$host"; fi
    rm -rf -- "$owned"
    return "$relay_failure"
}

case "$action" in
start)
    [ ! -e "$owned" ]
    [ "$(cat "$fixture/account")" = "cm1$token" ]
    ! ip netns list | awk '{print $1}' | grep -Fxq "$namespace"
    ! ip link show "$host" >/dev/null 2>&1
    ! ip route show exact 198.18.223.0/30 | grep -q .
    mkdir -m 700 "$owned"
    printf '%s\n' "$token" > "$owned/owner"
    trap 'cleanup' EXIT
    ip netns add "$namespace"
    ip link add "$host" type veth peer name "$peer"
    ip link set "$peer" netns "$namespace"
    ip address add 198.18.223.1/30 dev "$host"
    ip link set "$host" up
    ip netns exec "$namespace" ip link set lo up
    ip netns exec "$namespace" ip address add 198.18.223.2/30 dev "$peer"
    ip netns exec "$namespace" ip link set "$peer" up
    tc qdisc add dev "$host" root netem delay "${delay}ms" limit 4096
    ip netns exec "$namespace" tc qdisc add dev "$peer" root netem delay "${delay}ms" limit 4096
    # Replace only the just-created fixture daemon/config, before any clients.
    pid=$(cat "$fixture/sshd.pid")
    case "$pid" in *[!0-9]*|'') exit 2 ;; esac
    server=$(cat "$fixture/server")
    [ "$(readlink "/proc/$pid/exe")" = "$server" ]
    tr '\000' '\n' < "/proc/$pid/cmdline" | grep -Fq "$fixture/sshd.conf"
    kill "$pid"
    cp "$fixture/sshd.conf" "$fixture/sshd-loopback.conf"
    sed 's/^ListenAddress 127\.0\.0\.1$/ListenAddress 198.18.223.2/' "$fixture/sshd-loopback.conf" > "$fixture/sshd.conf"
    if [ -d "$fixture/runtime" ]; then export LD_LIBRARY_PATH="$fixture/runtime/lib/x86_64-linux-gnu:$fixture/runtime/usr/lib/x86_64-linux-gnu"; fi
    nohup ip netns exec "$namespace" "$server" -D -f "$fixture/sshd.conf" </dev/null >/dev/null 2>&1 &
    printf '%s\n' "$!" > "$fixture/sshd.pid"
    port=$(awk '/^Port / {print $2}' "$fixture/sshd.conf")
    nohup python3 "$proxy" "$port" 198.18.223.2 --bind "$bind" --owner "$token" --send-buffer-bytes "$send_buffer" --receive-buffer-bytes "$receive_buffer" </dev/null >"$owned/proxy.stdout" 2>"$owned/proxy.stderr" &
    printf '%s\n' "$!" > "$owned/proxy.pid"
    printf '%s\n' "$token" > "$owned/proxy-owner"
    printf '%s\n' "$bind" > "$owned/proxy-bind"
    sleep 1
    kill -0 "$(cat "$fixture/sshd.pid")"
    kill -0 "$(cat "$owned/proxy.pid")"
    ping -n -c 8 -i 0.2 -W 2 198.18.223.2 > "$owned/rtt.txt"
    tc -s qdisc show dev "$host" > "$owned/host-qdisc.txt"
    ip netns exec "$namespace" tc -s qdisc show dev "$peer" > "$owned/peer-qdisc.txt"
    trap - EXIT
    printf '%s\n' 'NETWORK_STARTED'
    ;;
observe)
    [ "$(cat "$owned/owner")" = "$token" ]
    cat "$owned/rtt.txt"
    cat "$owned/host-qdisc.txt" "$owned/peer-qdisc.txt"
    ;;
stop)
    cleanup
    printf '%s\n' 'NETWORK_CLEANED'
    ;;
*) exit 2 ;;
esac
