#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "$0")/.."
unset SSH_AGENT_PID SSH_AUTH_SOCK
token=$(openssl rand -hex 6)
workspace="/tmp/capstan-m1-sshd-$token"
secrets="$workspace/secrets"
fixture="/var/tmp/capstan-m1-ssh-$token"
account="cm1$token"
port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
mkdir -m 700 "$workspace"

run_fixture() {
    sudo -- sh scripts/m1-sshd-fixture.sh "$1" "$fixture" "$secrets" "$port" "$account" none ed25519 sftp
}

cleanup() {
    local status=$?
    trap - EXIT
    if [[ -n ${SSH_AGENT_PID:-} ]]; then ssh-agent -k >/dev/null || status=1; fi
    if [[ -d $fixture ]]; then run_fixture stop || status=1; fi
    if [[ -d $secrets ]]; then
        rm -f -- "$secrets"/* || status=1
        rmdir -- "$secrets" || status=1
    fi
    rmdir -- "$workspace" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -m 700 "$secrets"
run_fixture start
sudo -- chown "$(id -u):$(id -g)" "$secrets/password.txt" "$secrets/openssh" "$secrets/host-key.pub"
eval "$(ssh-agent -s)" >/dev/null
ssh-add "$secrets/openssh" 2>/dev/null

export CAPSTAN_TEST_SSHD="$account@127.0.0.1:$port"
export CAPSTAN_TEST_SSH_PASSWORD_FILE="$secrets/password.txt"
export CAPSTAN_TEST_SSH_KEY_FILE="$secrets/openssh"
export CAPSTAN_TEST_SSH_HOST_KEY_FILE="$secrets/host-key.pub"
export CAPSTAN_TEST_SSH_HOST_FINGERPRINT
CAPSTAN_TEST_SSH_HOST_FINGERPRINT=$(ssh-keygen -lf "$CAPSTAN_TEST_SSH_HOST_KEY_FILE" | awk '{print $2}')
export CAPSTAN_TEST_SSH_AGENT=openssh
export CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY="$fixture/home/forwarding.pub"
export CAPSTAN_TEST_SFTP_ROOT="$fixture/home"

mkdir -p target/ci-openssh
run_probe() {
    local package=$1 suite=$2 case_name=$3
    local log="target/ci-openssh/$suite.log"
    cargo test --locked -p "$package" --test "$suite" "$case_name" -- --exact --nocapture 2>&1 | tee "$log"
    if grep -q 'SKIP:' "$log" || ! grep -q '^test result: ok. 1 passed;' "$log"; then
        printf 'Required OpenSSH probe did not run: %s\n' "$case_name" >&2
        return 1
    fi
}

run_probe capstan-ssh real_sshd configured_openssh_target_supports_authentication_pty_unicode_and_multiple_shells
grep -Fq 'Running real-sshd compatibility checks for 3 configured authentication method(s)' target/ci-openssh/real_sshd.log
run_probe capstan-ssh sftp_protocol configured_openssh_sftp_probe
run_probe capstan-core ssh_workflow configured_openssh_password_completes_delayed_trust_and_vault_restart_workflow
