import hashlib
import json
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
ASSET_NAMES = (*release.package_names(VERSION), release.SUMS)


class SourceTests(unittest.TestCase):
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
        self.github = GitHubFixture()
        patcher = patch.object(release, "github", side_effect=self.github.request)
        self.api = patcher.start()
        self.addCleanup(patcher.stop)

    def publish(self):
        release.publish(self.directory, VERSION, COMMIT)

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


if __name__ == "__main__":
    unittest.main()
