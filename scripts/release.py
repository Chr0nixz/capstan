"""Validate source versions and publish the verified Windows release assets."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


REPOSITORY = "Chr0nixz/capstan"
ROOT = Path(__file__).resolve().parent.parent
VERSION = re.compile(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){2}")
SHA = re.compile(r"[0-9a-f]{40}")
SUMS = "SHA256SUMS.txt"
LATEST = "latest.json"
MAX_PACKAGE = 200 * 1024 * 1024


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


def asset_hashes(directory, version, *, write_sums=False, with_updates=False):
    names = package_names(version)
    expected = set(names if write_sums else (*names, SUMS))
    if with_updates:
        expected.update((names[0] + ".sig", LATEST))
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


def nsis_payload_check(executable, config, output):
    """Embed build-time hashes, so later NSIS error-flag resets cannot hide refused writes."""
    bundle = json.loads(config.read_text(encoding="utf-8"))["bundle"]
    resources = bundle.get("resources", {})
    if not isinstance(resources, dict) or bundle.get("externalBin"):
        raise ValueError("Unsupported NSIS payload layout; extend verification before bundling.")
    files = {executable.name: executable}
    for source, target in resources.items():
        source = config.parent / source
        if source.is_symlink():
            raise ValueError("NSIS verification must not follow resource symlinks.")
        entries = sorted(source.rglob("*")) if source.is_dir() else [source]
        for entry in entries:
            if entry.is_symlink():
                raise ValueError("NSIS verification must not follow resource symlinks.")
            if entry.is_dir():
                continue
            suffix = entry.relative_to(source).as_posix() if source.is_dir() else ""
            relative = "/".join(part for part in (target.rstrip("/\\"), suffix) if part).replace("\\", "/")
            if (not relative or relative.startswith("/") or ":" in relative
                    or any(part in ("", ".", "..") for part in relative.split("/"))
                    or any(name.casefold() == relative.casefold() for name in files)):
                raise ValueError("Invalid or duplicate NSIS payload destination.")
            files[relative] = entry
    quote = lambda value: "'" + value.replace("'", "''") + "'"
    rows = [f"    @({quote(name)}, {quote(file_hash(path).upper())})" for name, path in sorted(files.items())]
    script = r"""param(
  [Parameter(Mandatory = $true)][string]$InstallDirectory,
  [ValidateSet('Verify', 'Prepare', 'Rollback')][string]$Mode = 'Verify',
  [string]$InventoryPath,
  [string]$BackupDirectory
)
$ErrorActionPreference = 'Stop'
try {
  $expected = @(
@PAYLOAD_ROWS@
  )
  $root = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
  function Resolve-PayloadPath([string]$base, [string]$relative) {
    if ([string]::IsNullOrEmpty($relative) -or [IO.Path]::IsPathRooted($relative) -or $relative.Contains(':')) { throw 'invalid_payload_path' }
    foreach ($part in $relative.Replace('\', '/').Split('/')) {
      if ($part -eq '' -or $part -eq '.' -or $part -eq '..') { throw 'invalid_payload_path' }
    }
    $path = [IO.Path]::GetFullPath([IO.Path]::Combine($base, $relative))
    if (-not $path.StartsWith($base, [StringComparison]::OrdinalIgnoreCase)) { throw 'invalid_payload_path' }
    $ancestor = $path
    while ($ancestor.Length -ge $base.TrimEnd([char[]]'\/').Length) {
      try {
        if (([IO.File]::GetAttributes($ancestor) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'payload_reparse_point' }
      } catch [IO.FileNotFoundException] { } catch [IO.DirectoryNotFoundException] { }
      $ancestor = [IO.Path]::GetDirectoryName($ancestor)
    }
    return $path
  }
  $files = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  $directories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($entry in $expected) {
    $null = Resolve-PayloadPath $root $entry[0]
    $null = $files.Add($entry[0])
    $parent = [IO.Path]::GetDirectoryName($entry[0])
    while (-not [string]::IsNullOrEmpty($parent)) {
      $null = $directories.Add($parent.Replace('\', '/'))
      $parent = [IO.Path]::GetDirectoryName($parent)
    }
  }
  # Tauri generates this file instead of copying it from the resource mapping.
  # It is managed too, but has no build-time digest to include in Verify mode.
  $null = $files.Add('uninstall.exe')
  if ($Mode -eq 'Prepare') {
    $added = [Collections.Generic.List[string]]::new()
    foreach ($set in @(@('F', $files), @('D', $directories))) {
      foreach ($relative in $set[1]) {
        $path = Resolve-PayloadPath $root $relative
        if (-not [IO.File]::Exists($path) -and -not [IO.Directory]::Exists($path)) {
          $added.Add($set[0] + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($relative)))
        }
      }
    }
    [IO.File]::WriteAllLines($InventoryPath, $added, [Text.UTF8Encoding]::new($false))
    exit 0
  }
  if ($Mode -eq 'Rollback') {
    $backup = [IO.Path]::GetFullPath($BackupDirectory).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
    if (-not [IO.Directory]::Exists($backup)) { throw 'backup_missing' }
    $removeFiles = [Collections.Generic.List[string]]::new()
    $removeDirectories = [Collections.Generic.List[string]]::new()
    # Validate every record before deleting anything. A modified inventory cannot
    # expand the embedded payload or remove a path present in the old backup.
    foreach ($line in [IO.File]::ReadAllLines($InventoryPath)) {
      if ($line.Length -lt 2) { throw 'invalid_inventory' }
      $relative = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($line.Substring(1)))
      $kind = $line.Substring(0, 1)
      if (($kind -eq 'F' -and -not $files.Contains($relative)) -or
          ($kind -eq 'D' -and -not $directories.Contains($relative)) -or
          ($kind -ne 'F' -and $kind -ne 'D')) { throw 'invalid_inventory' }
      $path = Resolve-PayloadPath $root $relative
      $old = Resolve-PayloadPath $backup $relative
      if ([IO.File]::Exists($old) -or [IO.Directory]::Exists($old)) { continue }
      if ($kind -eq 'F') { $removeFiles.Add($path) } else { $removeDirectories.Add($path) }
    }
    # Validate the entire backup tree before restoring it. Do not follow links
    # or let a locked, already-original file abort restoration of later files.
    $restoreFiles = [Collections.Generic.List[object]]::new()
    $restoreDirectories = [Collections.Generic.List[string]]::new()
    $pending = [Collections.Generic.Queue[string]]::new()
    $pending.Enqueue($backup)
    while ($pending.Count -gt 0) {
      foreach ($entry in [IO.Directory]::GetFileSystemEntries($pending.Dequeue())) {
        $relative = $entry.Substring($backup.Length).Replace('\', '/')
        $source = Resolve-PayloadPath $backup $relative
        $destination = Resolve-PayloadPath $root $relative
        if ([IO.Directory]::Exists($source)) {
          $restoreDirectories.Add($destination)
          $pending.Enqueue($source)
        } else { $restoreFiles.Add(@($source, $destination)) }
      }
    }
    $restoreFailed = $false
    $algorithm = [Security.Cryptography.SHA256]::Create()
    function Read-RollbackHash([string]$path) {
      $stream = [IO.File]::OpenRead($path)
      try { return [Convert]::ToBase64String($algorithm.ComputeHash($stream)) }
      finally { $stream.Dispose() }
    }
    try {
      foreach ($directory in @($root) + $restoreDirectories.ToArray()) {
        try { $null = [IO.Directory]::CreateDirectory($directory) }
        catch { $restoreFailed = $true }
      }
      foreach ($pair in $restoreFiles) {
        try {
          if ([IO.File]::Exists($pair[1]) -and
              (Read-RollbackHash $pair[0]) -ceq (Read-RollbackHash $pair[1])) { continue }
          [IO.File]::Copy($pair[0], $pair[1], $true)
        } catch { $restoreFailed = $true }
      }
    } finally { $algorithm.Dispose() }
    foreach ($path in $removeFiles) {
      if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) }
    }
    # Deepest directories first, and never recursive: unmanaged user files win.
    $removeDirectories.Sort([Comparison[string]]{ param($left, $right) $right.Length.CompareTo($left.Length) })
    foreach ($path in $removeDirectories) {
      if ([IO.Directory]::Exists($path) -and [IO.Directory]::GetFileSystemEntries($path).Length -eq 0) {
        [IO.Directory]::Delete($path, $false)
      }
    }
    if ($restoreFailed) { throw 'backup_restore_failed' }
    exit 0
  }
  $sha256 = [Security.Cryptography.SHA256]::Create()
  try {
    foreach ($entry in $expected) {
      $path = Resolve-PayloadPath $root $entry[0]
      $stream = [IO.File]::OpenRead($path)
      try { $hash = [BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '') }
      finally { $stream.Dispose() }
      if ($hash -cne $entry[1]) { exit 1 }
    }
  } finally { $sha256.Dispose() }
  exit 0
} catch { exit 1 }
""".replace('@PAYLOAD_ROWS@', ',\n'.join(rows))
    # Windows PowerShell 5.1 requires a BOM for non-ASCII resource names.
    output.write_text(script, encoding="utf-8-sig", newline="\n")


def public_key(root):
    key = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8")).get("plugins", {}).get("updater", {}).get("pubkey", "")
    if not key or len(key) > 4096:
        raise ValueError("Configure the maintainer's updater public key before publishing.")
    return key


def verify_signature(asset, encoded_signature, encoded_key, version):
    """Verify Tauri/minisign's artifact and global Ed25519 signatures with OpenSSL.

    No private key is needed. Never trust the comment's version until both signatures pass.
    """
    try:
        key_lines = base64.b64decode(encoded_key, validate=True).decode("utf-8").splitlines()
        signature_lines = base64.b64decode(encoded_signature.strip(), validate=True).decode("utf-8").splitlines()
        key = base64.b64decode(key_lines[1], validate=True)
        signature = base64.b64decode(signature_lines[1], validate=True)
        global_signature = base64.b64decode(signature_lines[3], validate=True)
        if (len(key_lines) != 2 or len(signature_lines) != 4 or len(key) != 42 or len(signature) != 74
                or len(global_signature) != 64 or key[:2] != b"Ed" or signature[:2] != b"ED"
                or key[2:10] != signature[2:10] or not signature_lines[2].startswith("trusted comment: ")):
            raise ValueError("Invalid updater signature encoding.")
        comment = signature_lines[2][len("trusted comment: "):]
        with tempfile.TemporaryDirectory(prefix="capstan-verify-") as temporary:
            directory = Path(temporary)
            # RFC 8410 SubjectPublicKeyInfo for an Ed25519 public key.
            (directory / "key.der").write_bytes(bytes.fromhex("302a300506032b6570032100") + key[10:])
            with asset.open("rb") as stream:
                digest = hashlib.file_digest(stream, lambda: hashlib.blake2b(digest_size=64)).digest()
            for message, sig in ((digest, signature[10:]), (signature[10:] + comment.encode("utf-8"), global_signature)):
                (directory / "message").write_bytes(message)
                (directory / "signature").write_bytes(sig)
                check = subprocess.run([
                    "openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER", "-inkey", str(directory / "key.der"),
                    "-rawin", "-in", str(directory / "message"), "-sigfile", str(directory / "signature"),
                ], capture_output=True, timeout=30, check=False)
                if check.returncode != 0:
                    raise ValueError("Updater signature verification failed.")
        if [field for field in comment.split("\t") if field.startswith("version:")] != [f"version:{version}"]:
            raise ValueError("Updater signature version differs from the release.")
    except (IndexError, UnicodeError, TypeError) as error:
        raise ValueError("Invalid updater signature encoding.") from error


def update_manifest(directory, version, key):
    installer, _ = package_names(version)
    path = directory / installer
    size = path.stat().st_size
    if not 0 < size <= MAX_PACKAGE:
        raise ValueError("Updater installer exceeds its size limit.")
    signature_path = directory / (installer + ".sig")
    if signature_path.is_symlink() or not 0 < signature_path.stat().st_size <= 4096:
        raise ValueError("Invalid updater signature file.")
    signature = signature_path.read_text(encoding="utf-8").strip()
    verify_signature(path, signature, key, version)
    return {"version": version, "notes": f"Capstan {version}", "platforms": {"windows-x86_64": {
        "url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{installer}",
        "signature": signature, "size": size,
    }}}


def sign_updates(directory, version, root=ROOT):
    before = asset_hashes(directory, version)
    key = public_key(root)
    installer, _ = package_names(version)
    # The private key and password are read from the protected job's environment,
    # never arguments, files in the checkout, or build/PR job outputs.
    subprocess.run(["pnpm.cmd" if os.name == "nt" else "pnpm", "-C", str(root / "ui"), "tauri", "signer", "sign",
                    "--app-version", version, str((directory / installer).resolve())], check=True)
    if file_hash(directory / installer) != before[installer]:
        raise ValueError("Signing changed the validated installer.")
    manifest = update_manifest(directory, version, key)
    with (directory / LATEST).open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(manifest, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def signed_asset_hashes(directory, version, root=ROOT):
    hashes = asset_hashes(directory, version, with_updates=True)
    expected = update_manifest(directory, version, public_key(root))
    latest = directory / LATEST
    if latest.is_symlink() or not 0 < latest.stat().st_size <= 65536 or json.loads(latest.read_text(encoding="utf-8")) != expected:
        raise ValueError("Invalid updater manifest.")
    for name in (package_names(version)[0] + ".sig", LATEST):
        hashes[name] = file_hash(directory / name)
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


def publish(directory, version, commit, root=ROOT):
    if not SHA.fullmatch(commit):
        raise ValueError("Invalid verified release commit.")
    hashes = signed_asset_hashes(directory, version, root)
    tag = f"v{version}"
    verify_remote_tag(tag, commit)
    marker = f"<!-- capstan-ci: {commit} -->"
    release = find_release(tag)
    if release is None:
        release = github("POST", "releases", payload={
            "tag_name": tag,
            "target_commitish": commit,
            "name": f"Capstan {tag}",
            "body": f"{marker}\n\nWindows x64 installer and portable ZIP. The installer carries a Tauri updater signature. "
                    "Windows Authenticode signing is not provided. Verify manual downloads using `SHA256SUMS.txt`.\n\n",
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
    payload = actions.add_parser("nsis-payload-check")
    payload.add_argument("executable", type=Path)
    payload.add_argument("config", type=Path)
    payload.add_argument("output", type=Path)
    for action in ("checksums", "sign-updates", "publish"):
        assets = actions.add_parser(action)
        assets.add_argument("directory", type=Path)
        assets.add_argument("version")
        if action == "publish":
            assets.add_argument("commit")
    args = parser.parse_args()
    if args.action == "version":
        print(app_version(ROOT))
    elif args.action == "nsis-payload-check":
        nsis_payload_check(args.executable, args.config, args.output)
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
        if args.action == "sign-updates":
            sign_updates(args.directory, args.version)
        else:
            publish(args.directory, args.version, args.commit)


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Release error: {error}", file=sys.stderr)
        sys.exit(1)
