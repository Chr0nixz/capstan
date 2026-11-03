"""Disposable synthetic SSH output fixture; accepts only probes and exit."""
import os
import sys
import threading

if sys.argv[1:] not in (["--bulk"], ["--idle"]):
    raise SystemExit(2)

stopped = threading.Event()
output_lock = threading.Lock()


def emit(value):
    with output_lock:
        view = memoryview(value)
        while view:
            view = view[os.write(sys.stdout.fileno(), view):]


def produce():
    if sys.argv[1] == "--bulk":
        block = bytearray(10 if i % 80 == 79 else b"abcdefghijklmnopqrstuvwxyz"[i % 26] for i in range(16 * 1024))
        for sequence in range(100 * 1024 * 1024 // len(block)):
            if stopped.is_set():
                return
            block[:8] = f"{sequence:08x}".encode("ascii")
            emit(block)
    emit(b"\r\nCAPSTAN_M0_END\r\n")


writer = threading.Thread(target=produce)
writer.start()
while True:
    line = sys.stdin.buffer.readline(64)
    if not line or len(line) >= 64:
        break
    command = line.strip()
    if command == b"exit":
        break
    if command.startswith(b"probe:") and len(command) == 12 and command[6:].isdigit():
        emit(b"\r\nCAPSTAN_M0_PROBE:" + command[6:] + b"\r\n")
stopped.set()
writer.join()
