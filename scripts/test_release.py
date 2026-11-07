import hashlib
import base64
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import release


VERSION = "1.2.3"
TAG = f"v{VERSION}"
COMMIT = "c" * 40
ANNOTATED_TAG = "a" * 40
ASSET_NAMES = (*release.package_names(VERSION), release.SUMS, release.package_names(VERSION)[0] + ".sig", release.LATEST)


def test_signature(directory, asset, version=VERSION):
    """Generate disposable Ed25519 material at runtime; never keep test keys in Git."""
    private = directory / "disposable.key"
    public = directory / "disposable.der"
    subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(private)], check=True, capture_output=True)
    subprocess.run(["openssl", "pkey", "-in", str(private), "-pubout", "-outform", "DER", "-out", str(public)], check=True, capture_output=True)
    raw_key = b"Ed" + b"test-key" + public.read_bytes()[-32:]
    encoded_key = base64.b64encode(b"untrusted comment: disposable key\n" + base64.b64encode(raw_key) + b"\n").decode("ascii")
    comment = f"timestamp:1\tfile:{asset.name}\tversion:{version}".encode("utf-8")

    def sign(message):
        source = directory / "message"
        target = directory / "signature"
        source.write_bytes(message)
        subprocess.run(["openssl", "pkeyutl", "-sign", "-inkey", str(private), "-rawin", "-in", str(source), "-out", str(target)], check=True, capture_output=True)
        return target.read_bytes()

    signature = sign(hashlib.blake2b(asset.read_bytes(), digest_size=64).digest())
    global_signature = sign(signature + comment)
    encoded_signature = base64.b64encode(b"untrusted comment: disposable signature\n" + base64.b64encode(b"ED" + b"test-key" + signature)
        + b"\ntrusted comment: " + comment + b"\n" + base64.b64encode(global_signature) + b"\n").decode("ascii")
    return encoded_key, encoded_signature


class SourceTests(unittest.TestCase):
    def test_rollback_removes_only_payload_additions_and_preserves_user_paths(self):
        for keep_late_user_file in (False, True):
            with self.subTest(keep_late_user_file=keep_late_user_file), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                payload = root / "payload"
                (payload / "runtime/new/deep").mkdir(parents=True)
                (payload / "runtime/empty-before").mkdir()
                (payload / "capstan-app.exe").write_bytes(b"new executable")
                (payload / "runtime/existing.dll").write_bytes(b"new runtime")
                (payload / "runtime/new-version-only.dll").write_bytes(b"new addition")
                (payload / "runtime/new/deep/运行 时.dll").write_bytes(b"new nested addition")
                (payload / "runtime/empty-before/added.dll").write_bytes(b"addition in old empty directory")
                (payload / "runtime/user-owned.dll").write_bytes(b"new managed replacement")
                installed = root / "installed"
                (installed / "runtime/empty-before").mkdir(parents=True)
                (installed / "capstan-app.exe").write_bytes(b"old executable")
                (installed / "runtime/existing.dll").write_bytes(b"old runtime")
                (installed / "runtime/user-owned.dll").write_bytes(b"original user DLL")
                (installed / "user-note.txt").write_bytes(b"unmanaged user data")
                backup = root / "installed.capstan-update-backup"
                shutil.copytree(installed, backup)
                original_files = {str(path.relative_to(installed)): path.read_bytes() for path in installed.rglob("*") if path.is_file()}
                original_directories = {str(path.relative_to(installed)) for path in installed.rglob("*") if path.is_dir()}
                config = root / "tauri.conf.json"
                config.write_text(json.dumps({"bundle": {"resources": {"payload/runtime/": "runtime/"}}}), encoding="utf-8")
                checker = root / "payload.ps1"
                inventory = root / "additions.lst"
                release.nsis_payload_check(payload / "capstan-app.exe", config, checker)
                def run(mode):
                    return subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(checker), str(installed), "-Mode", mode, "-InventoryPath", str(inventory), "-BackupDirectory", str(backup)], capture_output=True, check=False).returncode
                self.assertEqual(run("Prepare"), 0)
                shutil.copytree(payload, installed, dirs_exist_ok=True)
                (installed / "uninstall.exe").write_bytes(b"new generated uninstaller")
                if keep_late_user_file:
                    (installed / "runtime/new/user-note.txt").write_bytes(b"user file created after snapshot")
                self.assertEqual(run("Rollback"), 0)
                expected_files = dict(original_files)
                expected_directories = set(original_directories)
                if keep_late_user_file:
                    expected_files[str(Path("runtime/new/user-note.txt"))] = b"user file created after snapshot"
                    expected_directories.add(str(Path("runtime/new")))
                self.assertEqual({str(path.relative_to(installed)): path.read_bytes() for path in installed.rglob("*") if path.is_file()}, expected_files)
                self.assertEqual({str(path.relative_to(installed)) for path in installed.rglob("*") if path.is_dir()}, expected_directories)
                self.assertEqual(run("Rollback"), 0)  # Both NSIS failure callbacks may run.

    def test_rollback_continues_past_locked_files_and_reports_incomplete_restore(self):
        for unchanged_uninstaller in (True, False):
            with self.subTest(unchanged_uninstaller=unchanged_uninstaller), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                payload = root / "payload"
                (payload / "runtime").mkdir(parents=True)
                (payload / "capstan-app.exe").write_bytes(b"new executable")
                (payload / "runtime/existing.dll").write_bytes(b"new runtime")
                (payload / "runtime/added.dll").write_bytes(b"new addition")
                installed = root / "installed"
                (installed / "runtime").mkdir(parents=True)
                (installed / "capstan-app.exe").write_bytes(b"old executable")
                (installed / "runtime/existing.dll").write_bytes(b"old runtime")
                uninstaller = installed / "uninstall.exe"
                uninstaller.write_bytes(b"old uninstaller")
                backup = root / "backup"
                shutil.copytree(installed, backup)
                config = root / "tauri.conf.json"
                config.write_text(json.dumps({"bundle": {"resources": {"payload/runtime/": "runtime/"}}}), encoding="utf-8")
                checker = root / "payload.ps1"
                inventory = root / "inventory.lst"
                release.nsis_payload_check(payload / "capstan-app.exe", config, checker)
                command = ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(checker), str(installed), "-InventoryPath", str(inventory), "-BackupDirectory", str(backup), "-Mode"]
                self.assertEqual(subprocess.run([*command, "Prepare"], capture_output=True, check=False).returncode, 0)
                shutil.copytree(payload, installed, dirs_exist_ok=True)
                if not unchanged_uninstaller:
                    uninstaller.write_bytes(b"damaged and locked uninstaller")
                environment = {**os.environ, "CAPSTAN_TEST_LOCK_PATH": str(uninstaller)}
                lock_source = "$ErrorActionPreference='Stop'; $stream=[IO.File]::Open($env:CAPSTAN_TEST_LOCK_PATH,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read); try { [Console]::WriteLine('locked'); $null=[Console]::ReadLine() } finally { $stream.Dispose() }"
                lock = subprocess.Popen(["powershell", "-NoProfile", "-NonInteractive", "-Command", lock_source], env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                try:
                    self.assertEqual(lock.stdout.readline().strip(), "locked")
                    result = subprocess.run([*command, "Rollback"], capture_output=True, check=False)
                    if unchanged_uninstaller:
                        self.assertEqual(result.returncode, 0)
                        self.assertEqual(uninstaller.read_bytes(), b"old uninstaller")
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertEqual(uninstaller.read_bytes(), b"damaged and locked uninstaller")
                    self.assertEqual((installed / "capstan-app.exe").read_bytes(), b"old executable")
                    self.assertEqual((installed / "runtime/existing.dll").read_bytes(), b"old runtime")
                    self.assertFalse((installed / "runtime/added.dll").exists())
                    self.assertEqual((backup / "uninstall.exe").read_bytes(), b"old uninstaller")
                finally:
                    lock.communicate("\n", timeout=10)
                self.assertEqual(lock.returncode, 0)

    def test_rollback_inventory_cannot_expand_payload_or_delete_old_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "payload"
            payload.mkdir()
            (payload / "added.dll").write_bytes(b"added")
            executable = payload / "capstan-app.exe"
            executable.write_bytes(b"executable")
            installed = root / "installed"
            installed.mkdir()
            (installed / "capstan-app.exe").write_bytes(b"old executable")
            (installed / "user-note.txt").write_bytes(b"user data")
            backup = root / "installed.capstan-update-backup"
            shutil.copytree(installed, backup)
            config = root / "tauri.conf.json"
            config.write_text(json.dumps({"bundle": {"resources": {"payload/added.dll": "added.dll"}}}), encoding="utf-8")
            checker = root / "payload.ps1"
            inventory = root / "additions.lst"
            release.nsis_payload_check(executable, config, checker)
            def run(mode):
                return subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(checker), str(installed), "-Mode", mode, "-InventoryPath", str(inventory), "-BackupDirectory", str(backup)], capture_output=True, check=False).returncode
            self.assertEqual(run("Prepare"), 0)
            (installed / "added.dll").write_bytes(b"added")
            original = inventory.read_text(encoding="utf-8")
            for forbidden in ("../outside", "user-note.txt"):
                inventory.write_text(original + "F" + base64.b64encode(forbidden.encode()).decode() + "\n", encoding="utf-8")
                self.assertNotEqual(run("Rollback"), 0)
                self.assertEqual((installed / "added.dll").read_bytes(), b"added")
                self.assertEqual((installed / "user-note.txt").read_bytes(), b"user data")
            # Even a forged, otherwise valid payload record cannot delete an old file.
            inventory.write_text(original + "F" + base64.b64encode(b"capstan-app.exe").decode() + "\n", encoding="utf-8")
            self.assertEqual(run("Rollback"), 0)
            self.assertEqual((installed / "capstan-app.exe").read_bytes(), b"old executable")
            self.assertFalse((installed / "added.dll").exists())

    def test_payload_checker_uses_actual_bytes_and_preserves_user_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            resources = root / "resources"
            resources.mkdir()
            (resources / "运行 时.dll").write_bytes(b"runtime")
            executable = root / "capstan-app.exe"
            executable.write_bytes(b"executable")
            config = root / "tauri.conf.json"
            config.write_text(json.dumps({"bundle": {"resources": {"resources/": "runtime/"}}}), encoding="utf-8")
            output = root / "verify.ps1"
            release.nsis_payload_check(executable, config, output)
            self.assertTrue(output.read_bytes().startswith(b"\xef\xbb\xbf"))
            destination = root / "installed"
            (destination / "runtime").mkdir(parents=True)
            (destination / "capstan-app.exe").write_bytes(b"executable")
            library = destination / "runtime/运行 时.dll"
            library.write_bytes(b"runtime")
            settings = destination / "settings.json"
            settings.write_bytes(b"unchanged")
            def check():
                return subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(output), str(destination)], capture_output=True, check=False).returncode
            self.assertEqual(check(), 0)
            library.write_bytes(b"changed")  # Same length still must fail.
            self.assertNotEqual(check(), 0)
            library.unlink()
            self.assertNotEqual(check(), 0)
            self.assertEqual(settings.read_bytes(), b"unchanged")

    def test_payload_layout_rejects_escaping_and_colliding_destinations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            executable = root / "capstan-app.exe"
            executable.write_bytes(b"executable")
            resource = root / "runtime.dll"
            resource.write_bytes(b"runtime")
            config = root / "tauri.conf.json"
            for destination in ["../outside", "C:/outside", "/outside", "CAPSTAN-APP.EXE", "runtime//file"]:
                config.write_text(json.dumps({"bundle": {"resources": {"runtime.dll": destination}}}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    release.nsis_payload_check(executable, config, root / "verify.ps1")

    def test_signature_matches_the_pinned_tauri_cli(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            key_path = directory / "disposable.key"
            asset = directory / "disposable.exe"
            asset.write_bytes(b"disposable package signed at test runtime")
            executable = "pnpm.cmd" if os.name == "nt" else "pnpm"
            prefix = [executable, "-C", str(release.ROOT / "ui"), "tauri", "signer"]
            environment = {name: value for name, value in os.environ.items() if not name.startswith("TAURI_SIGNING_")}
            subprocess.run([*prefix, "generate", "--ci", "-p", "", "-w", str(key_path)], env=environment, check=True, capture_output=True)
            environment.update(TAURI_SIGNING_PRIVATE_KEY=key_path.read_text(encoding="utf-8"), TAURI_SIGNING_PRIVATE_KEY_PASSWORD="")
            subprocess.run([*prefix, "sign", "--app-version", VERSION, str(asset)], env=environment, check=True, capture_output=True)
            signature = Path(str(asset) + ".sig").read_text(encoding="utf-8").strip()
            key = Path(str(key_path) + ".pub").read_text(encoding="utf-8").strip()
            release.verify_signature(asset, signature, key, VERSION)
            asset.write_bytes(b"tampered package")
            with self.assertRaisesRegex(ValueError, "verification failed"):
                release.verify_signature(asset, signature, key, VERSION)

    def test_manifest_versions_must_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src-tauri").mkdir()
            (root / "ui").mkdir()
            (root / "src-tauri/tauri.conf.json").write_text(json.dumps({"version": VERSION}), encoding="utf-8")
            ui = root / "ui/package.json"
            ui.write_text(json.dumps({"version": "1.2.4"}), encoding="utf-8")
            cargo = {"packages": [{"name": "capstan-app", "version": VERSION}]}
            with patch.object(release, "command", return_value=json.dumps(cargo)):
                with self.assertRaisesRegex(ValueError, "matching stable versions"):
                    release.app_version(root)
                ui.write_text(json.dumps({"version": VERSION}), encoding="utf-8")
                self.assertEqual(release.app_version(root), VERSION)

    def test_preflight_requires_matching_tag_on_main(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)

            def git(*args):
                return release.command(root, "git", "-c", "user.name=Release Test", "-c", "user.email=test@example.invalid", *args)

            git("init", "--initial-branch=main", "--quiet")
            git("-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "Main commit")
            main = git("rev-parse", "HEAD")
            git("update-ref", "refs/remotes/origin/main", main)
            git("-c", "tag.gpgsign=false", "tag", "-a", TAG, "-m", "Fixture tag")
            annotated = git("rev-parse", f"refs/tags/{TAG}")
            with patch.object(release, "app_version", return_value=VERSION):
                self.assertEqual(release.preflight(root, "refs/heads/main", main), (VERSION, main))
                self.assertEqual(release.preflight(root, f"refs/tags/{TAG}", annotated), (VERSION, main))
                with self.assertRaisesRegex(ValueError, "Expected release tag"):
                    release.preflight(root, "refs/tags/v9.9.9", main)
                git("switch", "-c", "unmerged", "--quiet")
                git("-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "Unmerged commit")
                unmerged = git("rev-parse", "HEAD")
                git("tag", "--force", TAG, unmerged)
                with self.assertRaisesRegex(ValueError, "history of origin/main"):
                    release.preflight(root, f"refs/tags/{TAG}", unmerged)


class GitHubFixture:
    def __init__(self):
        self.draft = None
        self.assets = {}
        self.uploads = []
        self.fail_upload = None
        self.bad_digest = None
        self.tag_commit = COMMIT
        self.published = 0

    def request(self, method, endpoint, *, payload=None, asset=None):
        if (method, endpoint) == ("GET", f"git/ref/tags/{TAG}"):
            return {"object": {"type": "tag", "sha": ANNOTATED_TAG}}
        if (method, endpoint) == ("GET", f"git/tags/{ANNOTATED_TAG}"):
            return {"object": {"type": "commit", "sha": self.tag_commit}}
        if (method, endpoint) == ("GET", "releases?per_page=100&page=1"):
            return [self.draft] if self.draft else []
        if (method, endpoint) == ("POST", "releases"):
            if not payload["draft"] or self.draft:
                raise AssertionError("must create a single draft")
            self.draft = {**payload, "id": 1, "html_url": "https://example.invalid/release"}
            return self.draft
        if (method, endpoint) == ("GET", "releases/1/assets?per_page=100"):
            return list(self.assets.values())
        if method == "POST" and endpoint.startswith("releases/1/assets?name="):
            if asset.name in self.assets:
                raise AssertionError("duplicate upload")
            if asset.name == self.fail_upload:
                raise ValueError("simulated upload failure")
            self.uploads.append(asset.name)
            item = {"name": asset.name, "id": len(self.uploads), "state": "uploaded"}
            if asset.name != self.bad_digest:
                item["digest"] = "sha256:" + hashlib.sha256(asset.read_bytes()).hexdigest()
            self.assets[asset.name] = item
            return item
        if method == "DELETE" and endpoint.startswith("releases/assets/"):
            asset_id = int(endpoint.rsplit("/", 1)[1])
            name = next(name for name, item in self.assets.items() if item["id"] == asset_id)
            del self.assets[name]
            return None
        if (method, endpoint) == ("PATCH", "releases/1"):
            self.published += 1
            self.draft.update(payload)
            return self.draft
        raise AssertionError(f"unexpected GitHub request {method} {endpoint}")


class PublishTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        for name in release.package_names(VERSION):
            (self.directory / name).write_bytes(name.encode("ascii"))
        release.asset_hashes(self.directory, VERSION, write_sums=True)
        key_directory = tempfile.TemporaryDirectory()
        self.addCleanup(key_directory.cleanup)
        root = Path(key_directory.name)
        self.root = root
        (root / "src-tauri").mkdir()
        key, signature = test_signature(root, self.directory / ASSET_NAMES[0])
        (root / "src-tauri/tauri.conf.json").write_text(json.dumps({"plugins": {"updater": {"pubkey": key}}}), encoding="utf-8")
        self.key = key
        (self.directory / (ASSET_NAMES[0] + ".sig")).write_text(signature, encoding="utf-8")
        (self.directory / release.LATEST).write_text(json.dumps(release.update_manifest(self.directory, VERSION, key)), encoding="utf-8")
        self.github = GitHubFixture()
        patcher = patch.object(release, "github", side_effect=self.github.request)
        self.api = patcher.start()
        self.addCleanup(patcher.stop)

    def publish(self):
        release.publish(self.directory, VERSION, COMMIT, self.root)

    def test_missing_or_modified_asset_blocks_all_api_calls(self):
        asset = self.directory / ASSET_NAMES[0]
        asset.write_bytes(b"damaged package")
        with self.assertRaisesRegex(ValueError, "checksums do not match"):
            self.publish()
        asset.unlink()
        with self.assertRaisesRegex(ValueError, "missing release files"):
            self.publish()
        self.api.assert_not_called()

    def test_moved_tag_blocks_release_creation(self):
        self.github.tag_commit = "d" * 40
        with self.assertRaisesRegex(ValueError, "Remote release tag"):
            self.publish()
        self.assertIsNone(self.github.draft)

    def test_resume_partial_draft_and_preserve_published_release(self):
        self.github.fail_upload = ASSET_NAMES[1]
        with self.assertRaisesRegex(ValueError, "simulated upload failure"):
            self.publish()
        self.assertTrue(self.github.draft["draft"])
        self.assertEqual(self.github.uploads, [ASSET_NAMES[0]])

        self.github.fail_upload = None
        self.publish()
        self.assertFalse(self.github.draft["draft"])
        self.assertEqual(self.github.uploads, list(ASSET_NAMES))
        self.publish()
        self.assertEqual(self.github.uploads, list(ASSET_NAMES))
        self.assertEqual(self.github.published, 1)

    def test_draft_digest_mismatch_blocks_publish_and_is_retryable(self):
        self.github.bad_digest = release.SUMS
        with self.assertRaisesRegex(ValueError, "draft assets failed SHA-256"):
            self.publish()
        self.assertTrue(self.github.draft["draft"])
        self.github.bad_digest = None
        self.publish()
        self.assertEqual(self.github.uploads, [*ASSET_NAMES, release.SUMS])

    def test_draft_created_elsewhere_is_not_overwritten(self):
        self.github.draft = {"tag_name": TAG, "draft": True, "body": "Manual draft"}
        with self.assertRaisesRegex(ValueError, "not owned"):
            self.publish()
        self.assertEqual(self.github.uploads, [])

    def test_wrong_key_tampered_signature_and_version_block_all_api_calls(self):
        asset = self.directory / ASSET_NAMES[0]
        signature = (self.directory / (ASSET_NAMES[0] + ".sig")).read_text(encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "version differs"):
            release.verify_signature(asset, signature, self.key, "1.2.4")
        wrong_key, _ = test_signature(self.root, asset)
        with self.assertRaisesRegex(ValueError, "verification failed"):
            release.verify_signature(asset, signature, wrong_key, VERSION)
        manifest = self.directory / release.LATEST
        contents = json.loads(manifest.read_text(encoding="utf-8"))
        contents["platforms"]["windows-x86_64"]["url"] = "https://example.invalid/installer.exe"
        manifest.write_text(json.dumps(contents), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "Invalid updater manifest"):
            self.publish()
        self.api.assert_not_called()

    def test_missing_public_key_prevents_publish(self):
        (self.root / "src-tauri/tauri.conf.json").write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "public key"):
            self.publish()
        self.api.assert_not_called()


if __name__ == "__main__":
    unittest.main()
