"""Validate source versions and publish the verified Windows release assets."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


REPOSITORY = "Chr0nixz/capstan"
ROOT = Path(__file__).resolve().parent.parent
VERSION = re.compile(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){2}")
SHA = re.compile(r"[0-9a-f]{40}")
SUMS = "SHA256SUMS.txt"


def command(root, *args):
    return subprocess.check_output(args, cwd=root, encoding="utf-8").strip()


def app_version(root):
    metadata = json.loads(command(root, "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"))
    cargo_version = next(package["version"] for package in metadata["packages"] if package["name"] == "capstan-app")
    tauri_version = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))["version"]
    ui_version = json.loads((root / "ui/package.json").read_text(encoding="utf-8"))["version"]
    if not VERSION.fullmatch(cargo_version) or cargo_version != tauri_version or cargo_version != ui_version:
        raise ValueError(f"Expected matching stable versions. Cargo={cargo_version}, Tauri={tauri_version}, UI={ui_version}.")
    return cargo_version


def preflight(root, ref, event_sha):
    if not SHA.fullmatch(event_sha):
        raise ValueError("Invalid event commit.")
    commit = command(root, "git", "rev-parse", "HEAD")
    if command(root, "git", "rev-parse", f"{event_sha}^{{commit}}") != commit:
        raise ValueError("Checkout does not match the event commit.")
    version = app_version(root)
    if ref != "refs/heads/main":
        if ref != f"refs/tags/v{version}":
            raise ValueError(f"Expected release tag v{version}.")
        if command(root, "git", "rev-parse", f"{ref}^{{commit}}") != commit:
            raise ValueError("Release tag has moved since the event.")
        status = subprocess.run(
            ["git", "merge-base", "--is-ancestor", commit, "refs/remotes/origin/main"], cwd=root, check=False,
        )
        if status.returncode != 0:
            raise ValueError("Release commit must be in the history of origin/main.")
    return version, commit


def package_names(version):
    if not VERSION.fullmatch(version):
        raise ValueError("Expected a stable release version.")
    return f"Capstan_{version}_x64-setup.exe", f"capstan-{version}-win-x64.zip"


def file_hash(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"Missing or invalid release asset {path.name}.")
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def asset_hashes(directory, version, *, write_sums=False):
    names = package_names(version)
    expected = set(names if write_sums else (*names, SUMS))
    if {path.name for path in directory.iterdir()} != expected:
        raise ValueError("Unexpected or missing release files.")
    hashes = {name: file_hash(directory / name) for name in names}
    manifest = "".join(f"{digest}  {name}\n" for name, digest in hashes.items()).encode("ascii")
    sums = directory / SUMS
    if write_sums:
        with sums.open("xb") as stream:
            stream.write(manifest)
    hashes[SUMS] = file_hash(sums)
    if sums.read_bytes() != manifest:
        raise ValueError("Release asset checksums do not match SHA256SUMS.txt.")
    return hashes


def github(method, endpoint, *, payload=None, asset=None):
    host = "https://uploads.github.com" if asset else "https://api.github.com"
    headers = {"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"}
    if asset:
        headers["Content-Type"] = "application/octet-stream"
        data = asset.read_bytes()
    elif payload is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(payload).encode("utf-8")
    else:
        data = None
    request = Request(f"{host}/repos/{REPOSITORY}/{endpoint}", data, headers, method=method)
    request.add_unredirected_header("Authorization", f"Bearer {os.environ['GITHUB_TOKEN']}")
    try:
        with urlopen(request, timeout=60) as response:
            body = response.read()
            return json.loads(body) if body else None
    except HTTPError as error:
        raise ValueError(f"GitHub {method} {endpoint} failed with HTTP {error.code}.") from error


def verify_remote_tag(tag, commit):
    target = github("GET", f"git/ref/tags/{tag}")["object"]
    for _ in range(8):
        if target["type"] == "commit" and target["sha"] == commit:
            return
        if target["type"] != "tag" or not SHA.fullmatch(target["sha"]):
            break
        target = github("GET", f"git/tags/{target['sha']}")["object"]
    raise ValueError("Remote release tag does not point to the verified commit.")


def find_release(tag):
    page = 1
    matches = []
    while True:
        releases = github("GET", f"releases?per_page=100&page={page}")
        matches.extend(release for release in releases if release["tag_name"] == tag)
        if len(releases) < 100:
            break
        page += 1
    if len(matches) > 1:
        raise ValueError("Multiple releases exist for this tag.")
    return matches[0] if matches else None


def publish(directory, version, commit):
    if not SHA.fullmatch(commit):
        raise ValueError("Invalid verified release commit.")
    hashes = asset_hashes(directory, version)
    tag = f"v{version}"
    verify_remote_tag(tag, commit)
    marker = f"<!-- capstan-ci: {commit} -->"
    release = find_release(tag)
    if release is None:
        release = github("POST", "releases", payload={
            "tag_name": tag,
            "target_commitish": commit,
            "name": f"Capstan {tag}",
            "body": f"{marker}\n\nWindows x64 installer and portable ZIP. These builds are unsigned. "
                    "Verify your download using `SHA256SUMS.txt`.\n\n",
            "draft": True,
            "generate_release_notes": True,
        })
    if not release["draft"]:
        print(f"Release {tag} is already published: {release['html_url']}")
        return
    if marker not in (release.get("body") or ""):
        raise ValueError("Existing release draft is not owned by this commit's workflow.")

    route = f"releases/{release['id']}"
    assets = {item["name"]: item for item in github("GET", f"{route}/assets?per_page=100")}
    if assets.keys() - hashes.keys():
        raise ValueError("Existing draft contains unexpected assets.")
    for name, digest in hashes.items():
        existing = assets.get(name)
        if existing and existing["state"] == "uploaded" and existing.get("digest") == f"sha256:{digest}":
            continue
        if existing:
            github("DELETE", f"releases/assets/{existing['id']}")
        github("POST", f"{route}/assets?{urlencode({'name': name})}", asset=directory / name)

    assets = github("GET", f"{route}/assets?per_page=100")
    remote_hashes = {item["name"]: item.get("digest") for item in assets if item["state"] == "uploaded"}
    if len(assets) != len(hashes) or remote_hashes != {name: f"sha256:{digest}" for name, digest in hashes.items()}:
        raise ValueError("GitHub draft assets failed SHA-256 verification.")
    verify_remote_tag(tag, commit)
    published = github("PATCH", route, payload={"draft": False, "make_latest": "legacy"})
    print(f"Published {published['html_url']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    actions.add_parser("version")
    check = actions.add_parser("preflight")
    check.add_argument("ref")
    check.add_argument("sha")
    for action in ("checksums", "publish"):
        assets = actions.add_parser(action)
        assets.add_argument("directory", type=Path)
        assets.add_argument("version")
        if action == "publish":
            assets.add_argument("commit")
    args = parser.parse_args()
    if args.action == "version":
        print(app_version(ROOT))
    elif args.action == "preflight":
        version, commit = preflight(ROOT, args.ref, args.sha)
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(f"version={version}\ncommit={commit}\n")
    elif args.action == "checksums":
        asset_hashes(args.directory, args.version, write_sums=True)
    else:
        if (os.environ.get("GITHUB_REPOSITORY") != REPOSITORY
                or os.environ.get("GITHUB_EVENT_NAME") != "push"
                or os.environ.get("GITHUB_REF") != f"refs/tags/v{args.version}"):
            raise ValueError("Publishing requires a version tag push to the Capstan repository.")
        publish(args.directory, args.version, args.commit)


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Release error: {error}", file=sys.stderr)
        sys.exit(1)
