"""Exercise generated hooks in isolated real Bash/Zsh PTYs, never user homes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import subprocess
import tempfile
import time
from urllib.parse import quote

parser = argparse.ArgumentParser()
parser.add_argument("--fixtures", type=Path, required=True)
parser.add_argument("--evidence", type=Path, required=True)
parser.add_argument("--zsh", type=Path, required=True)
args = parser.parse_args()
args.evidence.mkdir(parents=True, exist_ok=True)
assert not (args.evidence / "report.json").exists(), "Do not overwrite shell evidence"
shells = {"bash": Path("/bin/bash"), "zsh": args.zsh.resolve()}
prompt = re.compile(rb"__CAPSTAN_PROMPT__:(\d+) ")
osc = re.compile(rb"\x1b\]7;(.*?)\x1b\\")
workspace = Path(tempfile.mkdtemp(prefix="capstan-shell-"))
results = []


def read_prompt(fd):
    output = bytearray()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if select.select([fd], [], [], 0.1)[0]:
            try:
                data = os.read(fd, 4096)
            except OSError:
                break
            if not data:
                break
            output.extend(data)
            assert len(output) <= 65536, "Unbounded fixture output"
            if prompt.search(output):
                return bytes(output)
    raise AssertionError("Timed out before the isolated prompt: " + repr(bytes(output[-2048:])))


def scenario(name, shell, login, variant):
    home = workspace / name
    home.mkdir(mode=0o700)
    cwd = home / "中文 空格%#'$"
    cwd.mkdir()
    script = (args.fixtures / (shell + ".sh")).resolve()
    env = {"HOME": str(home), "ZDOTDIR": str(home), "TERM": "xterm-256color", "PATH": "/usr/bin:/bin", "LC_ALL": "C.UTF-8", "CAPSTAN_FIXTURE_SCRIPT": str(script)}
    setup = "stty -echo\n"
    if shell == "bash":
        if variant == "array":
            setup += 'PROMPT_COMMAND=(\'printf "__OLD__:%s\\n" "$?"\')\nexport PROMPT_COMMAND\n'
        else:
            setup += 'PROMPT_COMMAND=\'printf "__OLD__:%s\\n" "$?"\'\n'
        if variant == "readonly":
            setup += "readonly PROMPT_COMMAND\n"
    else:
        setup += 'old_hook() { printf "__OLD__:%s\\n" "$?"; }\nprecmd_functions=(old_hook)\n'
        if variant == "readonly":
            setup += "readonly precmd_functions\n"
        if variant == "empty":
            setup += "unset precmd_functions\n"
    if variant == "nounset":
        setup += "set -u\n"
    setup += '. "$CAPSTAN_FIXTURE_SCRIPT"\n. "$CAPSTAN_FIXTURE_SCRIPT"\n'
    setup += "PS1='__CAPSTAN_PROMPT__:$? '\n" if shell == "bash" else "PS1='__CAPSTAN_PROMPT__:%? '\n"
    if shell == "bash":
        (home / ".bashrc").write_text(setup, encoding="utf-8")
        (home / ".bash_profile").write_text('. "$HOME/.bashrc"\n', encoding="utf-8")
        command = [str(shells[shell]), *(["--login"] if login else []), "-i"]
    else:
        modules = next(args.zsh.resolve().parents[1].glob("usr/lib/*/zsh/5.9"))
        env["CAPSTAN_ZSH_MODULES"] = str(modules)
        (home / ".zshenv").write_text('module_path=( "$CAPSTAN_ZSH_MODULES" )\n', encoding="utf-8")
        (home / ".zshrc").write_text(setup, encoding="utf-8")
        command = [str(shells[shell]), "-il" if login else "-i"]
    master, slave = pty.openpty()
    process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=env, cwd=cwd, start_new_session=True)
    os.close(slave)
    records = []
    try:
        records.append(read_prompt(master))
        os.write(master, b"false\n")
        failed_command = read_prompt(master)
        records.append(failed_command)
        expected = ("file://127.0.0.1" + quote(str(cwd), safe="/")).encode()
        hints = osc.findall(failed_command)
        active = variant != "readonly"
        assert hints == ([expected] if active else []), (name, "hint encoding/idempotence", hints)
        if variant != "empty":
            assert b"__OLD__:1\r\n" in failed_command, (name, "old hook lost original status", failed_command)
        assert prompt.search(failed_command).group(1) == b"1", (name, "prompt lost command status", failed_command)
        if active:
            for command in [b"PWD='relative'; false\n", b"PWD='/'; false\n", b"PWD=$(printf '%03000d' 0); PWD=/${PWD//0/ }; false\n"]:
                os.write(master, command)
                output = read_prompt(master)
                records.append(output)
                if command.startswith(b"PWD='/'"):
                    assert osc.findall(output) == [b"file://127.0.0.1/"], (name, "root path")
                else:
                    assert not osc.findall(output), (name, "oversize/relative hint emitted")
        os.write(master, b"exit 0\n")
        process.wait(timeout=5)
        assert process.returncode == 0
        return {"name": name, "shell": shell, "login": login, "variant": variant, "passed": True, "hintMatched": True, "coexistsWithOldHooks": variant != "empty", "outputSha256": hashlib.sha256(b"".join(records)).hexdigest()}
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        os.close(master)
        (args.evidence / (name + ".bin")).write_bytes(b"".join(records))


cleaned = False
failure = None
try:
    for shell in shells:
        for login in [False, True]:
            for variant in (["scalar", "array", "readonly", "nounset"] if shell == "bash" else ["array", "empty", "readonly", "nounset"]):
                name = f"{shell}-{'login' if login else 'interactive'}-{variant}"
                try:
                    results.append(scenario(name, shell, login, variant))
                except Exception as error:
                    results.append({"name": name, "passed": False, "error": str(error)})
    assert len(results) == 16 and all(case["passed"] for case in results), "One or more real shell cases failed"
except Exception as error:
    failure = str(error)
finally:
    # Only this fresh mkdtemp-owned fixture is removed, within Linux /tmp.
    assert workspace.parent == Path(tempfile.gettempdir()) and workspace.name.startswith("capstan-shell-")
    shutil.rmtree(workspace)
    cleaned = not workspace.exists()
    report = {"schemaVersion": 1, "passed": failure is None, "cases": results, "cleanedUp": cleaned, "failure": failure,
              "shells": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "version": subprocess.check_output([str(path), "--version"], text=True).splitlines()[0]} for name, path in shells.items()},
              "generatedScripts": {name: hashlib.sha256((args.fixtures / (name + ".sh")).read_bytes()).hexdigest() for name in shells},
              "limits": "Local isolated WSL PTYs only; no remote config transaction, application install entry or native SFTP follow acceptance"}
    (args.evidence / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if failure:
    raise SystemExit(failure)
print("16 real Bash/Zsh hook cases passed; isolated homes cleaned")
