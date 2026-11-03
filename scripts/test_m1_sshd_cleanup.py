"""Adversarial identity checks for disposable sshd process cleanup."""
import importlib.util
from dataclasses import replace
from pathlib import Path
import signal
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('m1_sshd_cleanup', Path(__file__).with_name('m1-sshd-cleanup.py'))
cleanup = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = cleanup
SPEC.loader.exec_module(cleanup)


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.fixture = Path('/var/tmp/capstan-m1-ssh-0123456789ab')
        self.scope = cleanup.Scope(self.fixture, 'cm10123456789ab', 1003, '/usr/sbin/sshd')

    def process(self, uid=0, executable='/usr/sbin/sshd', args=(), started='123', zombie=False):
        return cleanup.Process(200, uid, started, executable, args, zombie)

    def test_scoped_listener_and_orphan_privileged_monitor_are_owned(self):
        self.assertTrue(self.scope.owns(self.process(args=('sshd', '-D', '-f', str(self.fixture / 'sshd.conf')))))
        self.assertTrue(self.scope.owns(self.process(args=('sshd: cm10123456789ab [priv]',))))

    def test_openssh_rewritten_listener_title_is_owned(self):
        title = f'sshd: /usr/sbin/sshd -D -f {self.fixture / "sshd.conf"} [listener] 0 of 10-100 startups'
        self.assertTrue(self.scope.owns(self.process(args=(title,))))
        self.assertFalse(self.scope.owns(self.process(args=(title.replace('sshd.conf', 'sshd.conf-other'),))))

    def test_other_fixture_and_substring_matches_are_rejected(self):
        for args in (('sshd', '-f', str(self.fixture / 'sshd.conf') + '-other'),
                     ('sshd: cm10123456789ac [priv]',),
                     ('sshd: cm10123456789ab [priv] extra',),
                     ('sshd', '-f', '/etc/ssh/sshd_config')):
            with self.subTest(args=args):
                self.assertFalse(self.scope.owns(self.process(args=args)))

    def test_executable_and_uid_must_match_for_privileged_process(self):
        args = ('sshd: cm10123456789ab [priv]',)
        self.assertFalse(self.scope.owns(self.process(executable='/tmp/sshd', args=args)))
        self.assertFalse(self.scope.owns(self.process(uid=1000, args=args)))

    def test_numeric_uid_alone_never_grants_process_ownership(self):
        self.assertFalse(self.scope.owns(self.process(uid=1003, executable='/usr/bin/python3', args=('python3', 'benchmark.py'))))
        self.assertFalse(self.scope.owns(self.process(uid=1000, executable='/usr/bin/python3')))
        self.assertFalse(self.scope.owns(self.process(uid=1003, zombie=True)))
        no_account = cleanup.Scope(self.fixture, self.scope.account, None, self.scope.server)
        self.assertFalse(no_account.owns(self.process(uid=1003)))

    def test_token_bound_benchmark_and_authenticated_sessions_are_owned(self):
        for args in (('sshd: cm10123456789ab@pts/3,pts/4',), ('sshd: cm10123456789ab@internal-sftp',)):
            self.assertTrue(self.scope.owns(self.process(uid=1003, args=args)))
        args = ('python3', str(self.fixture / 'home/benchmark.py'), '--bulk')
        self.assertTrue(self.scope.owns(self.process(uid=1003, executable='/usr/bin/python3.12', args=args)))
        self.assertFalse(self.scope.owns(self.process(uid=1003, executable='/tmp/other', args=args)))
        self.assertFalse(self.scope.owns(self.process(uid=0, executable='/usr/bin/python3.12', args=args)))

    def test_descendants_are_proven_before_reparenting_and_uid_reuse_is_rejected(self):
        root = self.process(args=('sshd: cm10123456789ab [priv]',))
        child = replace(self.process(uid=1003, executable='/bin/sh'), pid=201, parent=200)
        grandchild = replace(child, pid=202, parent=201, started='124')
        unrelated = replace(child, pid=203, parent=1)
        identities = {}
        selected = cleanup.select_owned(self.scope, [grandchild, unrelated, child, root], identities)
        self.assertEqual({p.pid for p in selected}, {200, 201, 202})
        orphan = replace(grandchild, parent=1)
        selected = cleanup.select_owned(self.scope, [orphan, unrelated], identities)
        self.assertEqual([p.pid for p in selected], [202])
        reused = replace(orphan, started='125')
        self.assertEqual(cleanup.select_owned(self.scope, [reused, unrelated], identities), [])

    def test_unrelated_container_uid_and_older_child_are_rejected(self):
        root = self.process(args=('sshd: cm10123456789ab [priv]',))
        same_uid = replace(self.process(uid=1003, executable='/bin/sh'), pid=205, parent=200, started='122')
        other_uid = replace(same_uid, uid=1000, started='124')
        for process in (same_uid, other_uid):
            self.assertEqual(cleanup.select_owned(self.scope, [root, process], {}), [root])

    def test_invalid_fixture_paths_and_accounts_are_rejected_before_io(self):
        for fixture, account in (('/var/tmp/capstan-m1-ssh-../other', self.scope.account),
                                 (str(self.fixture), 'cm10123456789ac'),
                                 ('/var/tmp/capstan-m1-ssh-0123', 'cm10123')):
            with self.subTest(fixture=fixture, account=account):
                with self.assertRaises(ValueError):
                    cleanup.load_scope(fixture, account, lambda _: self.fail('Lookup must not run'))

    def load(self, *, account=None, uid=1003, home=None, server='/usr/sbin/sshd', marker=True, symlink=False):
        def text(path):
            if path.name == 'account':
                return account if account is not None else self.scope.account
            return server

        def exists(path):
            return path.name == 'server' or (path.name == 'created-account' and marker)

        with patch.object(cleanup.Path, 'is_symlink', return_value=symlink), \
             patch.object(cleanup.Path, 'is_dir', return_value=True), \
             patch.object(cleanup.Path, 'is_file', exists), \
             patch.object(cleanup.Path, 'read_text', text):
            return cleanup.load_scope(str(self.fixture), self.scope.account,
                                      lambda _: SimpleNamespace(pw_uid=uid, pw_dir=home or str(self.fixture / 'home')))

    def test_scope_validates_stored_account_home_uid_and_server(self):
        self.assertEqual(self.load(), self.scope)
        for values in ({'account': 'cm10123456789ac'}, {'uid': 0},
                       {'home': '/home/other'}, {'server': '/tmp/sshd'}, {'symlink': True}):
            with self.subTest(values=values), self.assertRaises(ValueError):
                self.load(**values)

    def test_missing_creation_marker_never_grants_account_process_ownership(self):
        scope = self.load(marker=False)
        self.assertIsNone(scope.uid)
        self.assertFalse(scope.owns(self.process(uid=1003)))

    def test_pid_reuse_or_exec_change_is_never_signalled(self):
        original = self.process(args=('sshd: cm10123456789ab [priv]',))
        replacements = (self.process(started='124'), self.process(executable='/usr/bin/other'))
        for current in replacements:
            with patch.object(cleanup.os, 'pidfd_open', return_value=17, create=True), \
                 patch.object(cleanup.os, 'close') as close, \
                 patch.object(cleanup, 'read_process', return_value=current), \
                 patch.object(cleanup.signal, 'pidfd_send_signal', create=True) as send:
                self.assertFalse(cleanup.signal_owned(self.scope, original, signal.SIGTERM))
                send.assert_not_called()
                close.assert_called_once_with(17)

    def test_matching_identity_uses_retained_pidfd(self):
        process = self.process(args=('sshd: cm10123456789ab [priv]',))
        with patch.object(cleanup.os, 'pidfd_open', return_value=17, create=True), \
             patch.object(cleanup.os, 'close') as close, \
             patch.object(cleanup, 'read_process', return_value=process), \
             patch.object(cleanup.signal, 'pidfd_send_signal', create=True) as send:
            self.assertTrue(cleanup.signal_owned(self.scope, process, signal.SIGTERM))
            send.assert_called_once_with(17, signal.SIGTERM)
            close.assert_called_once_with(17)


if __name__ == '__main__':
    unittest.main()
