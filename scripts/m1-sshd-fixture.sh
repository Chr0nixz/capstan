#!/bin/sh
# A disposable, loopback-only OpenSSH fixture. No system sshd configuration is changed.
set -eu

action=$1
fixture=$2
secrets=$3
port=$4
account=$5
pageant=${6:-none}
host_algorithm=${7:-ed25519}
measurement=${8:-none}
case "$host_algorithm" in ed25519|rsa) ;; *) exit 2 ;; esac
case "$measurement" in none|benchmark|sftp|benchmark-sftp) ;; *) exit 2 ;; esac

token=${account#cm1}
case "$token" in *[!0-9a-f]*) exit 2 ;; esac
[ "${#token}" -eq 12 ]
[ "$account" = "cm1$token" ]
[ "$fixture" = "/var/tmp/capstan-m1-ssh-$token" ]
[ ! -L "$fixture" ]
case "$secrets" in
    /mnt/*/capstan/target/m1-sshd-*/*) ;;
    "/tmp/capstan-m1-sshd-$token/secrets") [ "$(realpath -m -- "$secrets")" = "$secrets" ] ;;
    *) exit 2 ;;
esac
case "$port" in *[!0-9]*|'') exit 2 ;; esac
[ "$port" -ge 1024 ] && [ "$port" -le 65535 ]

stop_server() {
    # A disconnected client can leave authenticated sshd children and blocked
    # fixture writers alive after the listener exits. Reap the exact generated
    # account and privileged monitors, including when sshd.pid is already gone.
    python3 "$(dirname "$0")/m1-sshd-cleanup.py" "$fixture" "$account"
    if [ -f "$fixture/sshd.pid" ]; then
        rm -f "$fixture/sshd.pid"
    fi
}

case "$action" in
start)
    [ ! -e "$fixture" ]
    [ -d "$secrets" ]
    ! getent passwd "$account" >/dev/null
    mkdir -m 755 "$fixture"
    printf '%s\n' "$account" > "$fixture/account"
    mkdir -m 755 "$fixture/home"
    mkdir "$fixture/packages"
    if [ -x /usr/sbin/sshd ]; then
        server=/usr/sbin/sshd
    else
        cd "$fixture/packages"
        apt-get download openssh-server libwrap0 >/dev/null 2>&1
        for package in ./*.deb; do
            dpkg-deb -x "$package" "$fixture/runtime"
        done
        server="$fixture/runtime/usr/sbin/sshd"
        export LD_LIBRARY_PATH="$fixture/runtime/lib/x86_64-linux-gnu:$fixture/runtime/usr/lib/x86_64-linux-gnu"
    fi
    if ! getent passwd sshd >/dev/null; then
        useradd --system --no-create-home --home-dir /run/sshd --shell /usr/sbin/nologin --gid nogroup sshd
        touch "$fixture/created-sshd-user"
    fi
    if [ ! -d /run/sshd ]; then
        mkdir -m 755 /run/sshd
        touch "$fixture/created-run-sshd"
    fi
    useradd --no-create-home --home-dir "$fixture/home" --shell /bin/sh "$account"
    touch "$fixture/created-account"
    chown "$account:$account" "$fixture/home"
    umask 077
    openssl rand -hex 24 > "$secrets/password.txt"
    openssl rand -hex 24 > "$secrets/passphrase.txt"
    { printf '%s:' "$account"; cat "$secrets/password.txt"; } | chpasswd
    ssh-keygen -q -t ed25519 -N '' -f "$secrets/openssh"
    ssh-keygen -q -t ed25519 -N "$(cat "$secrets/passphrase.txt")" -f "$secrets/openssh-encrypted"
    ssh-keygen -q -t rsa -b 2048 -N '' -f "$secrets/rsa-openssh"
    ssh-keygen -q -t rsa -b 2048 -N "$(cat "$secrets/passphrase.txt")" -f "$secrets/rsa-openssh-encrypted"
    cp "$secrets/rsa-openssh" "$secrets/rsa-pkcs1"
    ssh-keygen -q -p -m PEM -P '' -N '' -f "$secrets/rsa-pkcs1" >/dev/null
    openssl pkcs8 -topk8 -nocrypt -in "$secrets/rsa-pkcs1" -out "$secrets/rsa-pkcs8"
    openssl pkcs8 -topk8 -in "$secrets/rsa-pkcs1" -out "$secrets/rsa-pkcs8-encrypted" -v2 aes-256-cbc -passout "file:$secrets/passphrase.txt"
    openssl genpkey -algorithm ED25519 -out "$secrets/pkcs8" 2>/dev/null
    openssl pkcs8 -topk8 -in "$secrets/pkcs8" -out "$secrets/pkcs8-encrypted" -v2 aes-256-cbc -passout "file:$secrets/passphrase.txt"
    { printf '\000\000\000\013ssh-ed25519\000\000\000\040'; openssl pkey -in "$secrets/pkcs8" -pubout -outform DER | tail -c 32; } | base64 -w 0 > "$fixture/pkcs8-base64"
    { printf 'ssh-ed25519 '; cat "$fixture/pkcs8-base64"; printf '\n'; } > "$secrets/pkcs8.pub"
    if [ "$pageant" = pageant ]; then
        cd "$fixture/packages"
        apt-get download putty-tools >/dev/null 2>&1
        for package in ./putty-tools_*.deb; do
            dpkg-deb -x "$package" "$fixture/runtime"
        done
        "$fixture/runtime/usr/bin/puttygen" "$secrets/openssh" -O private -o "$secrets/pageant.ppk"
    fi
    cat "$secrets/openssh.pub" "$secrets/openssh-encrypted.pub" "$secrets/pkcs8.pub" "$secrets/rsa-openssh.pub" "$secrets/rsa-openssh-encrypted.pub" > "$fixture/home/authorized_keys"
    cp "$secrets/openssh.pub" "$fixture/home/forwarding.pub"
    chown "$account:$account" "$fixture/home/authorized_keys" "$fixture/home/forwarding.pub"
    ssh-keygen -q -t "$host_algorithm" -N '' -f "$fixture/host-key"
    ssh-keygen -q -t "$host_algorithm" -N '' -f "$fixture/host-key-replacement"
    if [ "$measurement" = benchmark ] || [ "$measurement" = benchmark-sftp ]; then
        cp "$(dirname "$0")/m1-output-fixture.py" "$fixture/home/benchmark.py"
        chmod 644 "$fixture/home/benchmark.py"
        cp "$fixture/host-key.pub" "$secrets/host-key.pub"
        { uname -srmo; awk -F: '/model name/ {print $2; exit}' /proc/cpuinfo; } > "$secrets/server-environment.txt"
    fi
    if [ "$measurement" = sftp ] || [ "$measurement" = benchmark-sftp ]; then
        cp "$fixture/host-key.pub" "$secrets/host-key.pub"
    fi
    cat > "$fixture/sshd.conf" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $fixture/host-key
PidFile $fixture/sshd.pid
AuthorizedKeysFile $fixture/home/authorized_keys
PasswordAuthentication yes
PubkeyAuthentication yes
KbdInteractiveAuthentication no
PermitRootLogin no
AllowUsers $account
AllowAgentForwarding yes
X11Forwarding no
UsePAM no
LogLevel QUIET
EOF
    if [ "$measurement" = sftp ] || [ "$measurement" = benchmark-sftp ]; then
        # Pin the probe algorithms on the isolated server so the library and
        # WinSCP reference cannot silently negotiate different ciphers/KEX.
        printf '%s\n' 'Subsystem sftp internal-sftp' 'Ciphers aes256-gcm@openssh.com' 'KexAlgorithms curve25519-sha256' 'Compression no' >> "$fixture/sshd.conf"
    fi
    "$server" -t -f "$fixture/sshd.conf"
    printf '%s\n' "$server" > "$fixture/server"
    "$server" -V 2> "$secrets/server-version.txt"
    nohup "$server" -D -f "$fixture/sshd.conf" </dev/null >/dev/null 2>&1 &
    printf '%s\n' "$!" > "$fixture/sshd.pid"
    sleep 1
    kill -0 "$(cat "$fixture/sshd.pid")"
    printf '%s\n' 'STARTED'
    ;;
rotate)
    stop_server
    mv "$fixture/host-key" "$fixture/host-key-original"
    mv "$fixture/host-key.pub" "$fixture/host-key-original.pub"
    cp "$fixture/host-key-replacement" "$fixture/host-key"
    cp "$fixture/host-key-replacement.pub" "$fixture/host-key.pub"
    server=$(cat "$fixture/server")
    if [ -d "$fixture/runtime" ]; then
        export LD_LIBRARY_PATH="$fixture/runtime/lib/x86_64-linux-gnu:$fixture/runtime/usr/lib/x86_64-linux-gnu"
    fi
    nohup "$server" -D -f "$fixture/sshd.conf" </dev/null >/dev/null 2>&1 &
    printf '%s\n' "$!" > "$fixture/sshd.pid"
    sleep 1
    kill -0 "$(cat "$fixture/sshd.pid")"
    printf '%s\n' 'ROTATED'
    ;;
stop)
    if [ -d "$fixture" ]; then
        [ "$(cat "$fixture/account")" = "$account" ]
        stop_server
        if [ -f "$fixture/created-account" ]; then
            userdel "$account"
            groupdel "$account" 2>/dev/null || true
        fi
        if [ -f "$fixture/created-sshd-user" ]; then
            userdel sshd
        fi
        if [ -f "$fixture/created-run-sshd" ]; then
            rmdir /run/sshd
        fi
        rm -rf -- "$fixture"
    fi
    printf '%s\n' 'CLEANED'
    ;;
*) exit 2 ;;
esac
