"""Stop only processes owned by one disposable Linux OpenSSH fixture."""
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import signal
import sys
import time


@dataclass(frozen=True)
class Process:
    pid: int
    uid: int
    started: str
    executable: str
    arguments: tuple[str, ...]
    zombie: bool = False
    parent: int = 0


@dataclass(frozen=True)
class Scope:
    fixture: Path
    account: str
    uid: int | None
    server: str | None

    def owns(self, process):
        if process.zombie:
            return False
        # A container can reuse a numeric UID without an entry in host passwd.
        # UID alone is never ownership: require the token-bound sshd identity,
        # exact benchmark command, or a verified descendant of those processes.
        if process.uid not in (0, self.uid):
            return False
        args = process.arguments
        if process.uid == self.uid and len(args) == 3:
            if (args[1] == str(self.fixture / 'home/benchmark.py')
                    and args[2] in ('--bulk', '--idle')
                    and re.fullmatch(r'python3(?:\.\d+)?', Path(process.executable).name)):
                return True
        if process.executable != self.server:
            return False
        if len(args) == 1 and args[0].startswith('sshd: ') and ' [listener] ' in args[0]:
            # OpenSSH overwrites argv with its listener title after startup.
            # Fixture paths contain no whitespace; retain exact token matching.
            args = tuple(args[0].split())
        master = process.uid == 0 and any(
            args[index:index + 2] == ('-f', str(self.fixture / 'sshd.conf'))
            for index in range(len(args) - 1))
        title = ' '.join(args).strip()
        privileged = process.uid == 0 and title == f'sshd: {self.account} [priv]'
        session = (process.uid == self.uid and title.startswith(f'sshd: {self.account}@'))
        return master or privileged or session


def load_scope(fixture, account, lookup):
    fixture = Path(fixture)
    token = fixture.name.removeprefix('capstan-m1-ssh-')
    if (not re.fullmatch(r'[0-9a-f]{12}', token)
            or fixture != Path('/var/tmp') / f'capstan-m1-ssh-{token}'
            or account != f'cm1{token}' or fixture.is_symlink()):
        raise ValueError('Invalid disposable fixture scope')
    if not fixture.is_dir() or (fixture / 'account').read_text().strip() != account:
        raise ValueError('Fixture account identity mismatch')
    uid = None
    if (fixture / 'created-account').is_file():
        entry = lookup(account)
        if entry.pw_uid < 1000 or entry.pw_dir != str(fixture / 'home'):
            raise ValueError('Disposable account home/UID mismatch')
        uid = entry.pw_uid
    server_file = fixture / 'server'
    server = server_file.read_text().strip() if server_file.is_file() else None
    if server not in (None, '/usr/sbin/sshd', str(fixture / 'runtime/usr/sbin/sshd')):
        raise ValueError('Fixture server identity mismatch')
    return Scope(fixture, account, uid, server)


def read_process(pid):
    proc = Path('/proc') / str(pid)
    try:
        # comm can contain spaces/parentheses. Parse fields after the last ')'.
        fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
        uid = int((proc / 'status').read_text().split('Uid:\t', 1)[1].split()[0])
        arguments = tuple(value.decode() for value in (proc / 'cmdline').read_bytes().split(b'\0') if value)
        return Process(pid, uid, fields[19], os.readlink(proc / 'exe'), arguments,
                       fields[0] == 'Z', int(fields[1]))
    except (FileNotFoundError, ProcessLookupError):
        return None


def signal_owned(scope, process, sig, identities=None):
    # Retain a kernel process handle, then recheck identity. PID reuse can never
    # redirect a signal to a different process after this check.
    try:
        descriptor = os.pidfd_open(process.pid)
    except ProcessLookupError:
        return False
    try:
        current = read_process(process.pid)
        retained = identities is not None and identities.get(process.pid) == process.started
        if current != process or not (scope.owns(current) or retained):
            return False
        try:
            signal.pidfd_send_signal(descriptor, sig)
        except ProcessLookupError:
            return False
        return True
    finally:
        os.close(descriptor)


def select_owned(scope, processes, identities):
    selected = {process.pid: process for process in processes if not process.zombie
                and (scope.owns(process) or identities.get(process.pid) == process.started)}
    # Keep previously proven PID/start identities when killing the parent causes
    # reparenting; a reused PID or unrelated same-UID process never inherits them.
    while True:
        children = [process for process in processes
                    if process.pid not in selected and not process.zombie
                    and scope.uid is not None and process.uid == scope.uid
                    and process.parent in selected
                    and int(process.started) >= int(selected[process.parent].started)]
        if not children:
            break
        selected.update((process.pid, process) for process in children)
    identities.clear()
    identities.update((process.pid, process.started) for process in selected.values())
    return list(selected.values())


def owned_processes(scope, identities):
    processes = []
    for proc in Path('/proc').iterdir():
        if proc.name.isdecimal():
            process = read_process(int(proc.name))
            if process is not None:
                processes.append(process)
    return select_owned(scope, processes, identities)


def cleanup(scope):
    if not hasattr(os, 'pidfd_open') or not hasattr(signal, 'pidfd_send_signal'):
        raise RuntimeError('Fixture cleanup requires Linux pidfd support')
    started = time.monotonic()
    signalled = set()
    killed = set()
    identities = {}
    while True:
        remaining = owned_processes(scope, identities)
        if not remaining:
            print(json.dumps({'fixtureProcessCleanup': True, 'terminated': len(signalled), 'killed': len(killed)}))
            return
        elapsed = time.monotonic() - started
        if elapsed >= 6:
            raise RuntimeError('Owned fixture processes survived cleanup deadline')
        # Close the listener/privileged monitors before their descendants.
        for process in sorted(remaining, key=lambda item: item.uid):
            identity = (process.pid, process.started)
            sig = signal.SIGKILL if elapsed >= 2 else signal.SIGTERM
            sent = killed if sig == signal.SIGKILL else signalled
            if identity not in sent and signal_owned(scope, process, sig, identities):
                sent.add(identity)
        time.sleep(0.1)


if __name__ == '__main__':
    import pwd
    if len(sys.argv) != 3:
        raise SystemExit(2)
    cleanup(load_scope(sys.argv[1], sys.argv[2], pwd.getpwnam))
