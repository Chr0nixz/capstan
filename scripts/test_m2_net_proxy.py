"""Transport and lifecycle regressions for the measurement-only relay."""
import asyncio
from contextlib import redirect_stdout, suppress
import hashlib
import importlib.util
import io
import json
import re
import socket
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location("m2_net_proxy", Path(__file__).with_name("m2-net-proxy.py"))
proxy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proxy)


async def until(predicate):
    async def wait():
        while not predicate():
            await asyncio.sleep(0.01)
    await asyncio.wait_for(wait(), 5)


class RelayTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.servers = []
        self.relays = []
        self.streams = set()
        self.handlers = set()

    async def asyncTearDown(self):
        for relay in self.relays:
            await asyncio.wait_for(relay.close(), 3)
            self.assertEqual(relay.report()["activeClients"], 0)
            self.assertEqual(relay.report()["ownedWriters"], 0)
        for server in self.servers:
            server.close()
        for stream in self.streams:
            stream.transport.abort()
        pending = tuple(self.handlers)
        for task in pending:
            task.cancel()
        await asyncio.gather(*pending, return_exceptions=True)
        for server in self.servers:
            await asyncio.wait_for(server.wait_closed(), 3)

    async def endpoint(self, handler, send_buffer_bytes=0, receive_buffer_bytes=0):
        async def run(reader, writer):
            self.streams.add(writer)
            try:
                await handler(reader, writer)
            except (ConnectionError, OSError):
                pass
            finally:
                writer.close()
                with suppress(ConnectionError, OSError, asyncio.TimeoutError):
                    await asyncio.wait_for(writer.wait_closed(), 1)

        def accept(reader, writer):
            task = asyncio.create_task(run(reader, writer))
            self.handlers.add(task)
            task.add_done_callback(self.handlers.discard)

        server = await asyncio.start_server(accept, "127.0.0.1", 0, limit=proxy.CHUNK_BYTES)
        self.servers.append(server)
        relay = proxy.Relay("127.0.0.1", server.sockets[0].getsockname()[1], send_buffer_bytes, receive_buffer_bytes)
        self.relays.append(relay)
        return relay, await relay.start("127.0.0.1", 0)

    async def client(self, port):
        reader, writer = await asyncio.open_connection("127.0.0.1", port, limit=proxy.CHUNK_BYTES)
        self.streams.add(writer)
        return reader, writer

    async def test_bidirectional_large_streams_match_hashes_and_release(self):
        chunks = 128
        received = []

        async def digest(reader):
            result = hashlib.sha256()
            while data := await reader.read(proxy.CHUNK_BYTES):
                result.update(data)
            return result.digest()

        async def send(writer, value):
            for _ in range(chunks):
                writer.write(value * proxy.CHUNK_BYTES)
                await writer.drain()
            writer.write_eof()

        async def peer(reader, writer):
            result, _ = await asyncio.gather(digest(reader), send(writer, b"r"))
            received.append(result)

        relay, port = await self.endpoint(peer)
        reader, writer = await self.client(port)
        result, _ = await asyncio.wait_for(asyncio.gather(digest(reader), send(writer, b"s")), 10)
        await until(lambda: bool(received) and not relay.tasks)
        self.assertEqual(result, hashlib.sha256(b"r" * proxy.CHUNK_BYTES * chunks).digest())
        self.assertEqual(received[0], hashlib.sha256(b"s" * proxy.CHUNK_BYTES * chunks).digest())
        self.assertEqual(relay.forwarded_bytes, chunks * proxy.CHUNK_BYTES * 2)

    async def test_peer_reply_after_client_half_close_is_not_truncated(self):
        async def peer(reader, writer):
            message = await reader.read()
            writer.write(hashlib.sha256(message).digest())
            await writer.drain()

        relay, port = await self.endpoint(peer)
        reader, writer = await self.client(port)
        payload = b"half-close-regression" * 1000
        writer.write(payload)
        await writer.drain()
        writer.write_eof()
        result = await asyncio.wait_for(reader.read(), 3)
        self.assertEqual(result, hashlib.sha256(payload).digest())
        await until(lambda: not relay.tasks)

    async def test_slow_reader_does_not_block_a_sibling_and_shutdown_joins(self):
        started = asyncio.Event()
        completed = asyncio.Event()

        async def peer(reader, writer):
            if await reader.readexactly(1) == b"s":
                started.set()
                for _ in range(4096):
                    writer.write(b"x" * proxy.CHUNK_BYTES)
                    await writer.drain()
                completed.set()
            else:
                writer.write(await reader.readexactly(5))
                await writer.drain()

        relay, port = await self.endpoint(peer)
        _, blocked = await self.client(port)
        blocked.transport.pause_reading()
        blocked.write(b"s")
        await blocked.drain()
        await asyncio.wait_for(started.wait(), 3)
        reader, sibling = await self.client(port)
        sibling.write(b"ehello")
        await sibling.drain()
        self.assertEqual(await asyncio.wait_for(reader.readexactly(5), 3), b"hello")
        self.assertFalse(completed.is_set())
        await asyncio.wait_for(relay.close(), 3)
        self.assertFalse(relay.tasks)
        self.assertFalse(relay.writers)

    async def test_sixteen_client_cap_rejects_an_excess_connection(self):
        release = asyncio.Event()

        async def peer(reader, writer):
            await release.wait()

        relay, port = await self.endpoint(peer)
        for _ in range(proxy.MAX_CLIENTS):
            await self.client(port)
        await until(lambda: len(relay.tasks) == proxy.MAX_CLIENTS)
        reader, _ = await self.client(port)
        with suppress(ConnectionResetError):
            self.assertEqual(await asyncio.wait_for(reader.read(1), 3), b"")
        self.assertEqual(relay.rejected, 1)
        self.assertEqual(relay.accepted, proxy.MAX_CLIENTS)
        self.assertEqual(relay.peak_clients, proxy.MAX_CLIENTS)

    async def test_refused_peer_releases_client_admission(self):
        server = await asyncio.start_server(lambda r, w: w.close(), "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        server.close()
        await server.wait_closed()
        relay = proxy.Relay("127.0.0.1", port)
        self.relays.append(relay)
        local = await relay.start("127.0.0.1", 0)
        reader, _ = await self.client(local)
        self.assertEqual(await asyncio.wait_for(reader.read(), 3), b"")
        await until(lambda: not relay.tasks)
        self.assertFalse(relay.writers)

    async def test_explicit_send_buffer_applies_to_upstream_socket_only(self):
        release = asyncio.Event()

        async def peer(reader, writer):
            await release.wait()

        requested = proxy.CHUNK_BYTES
        relay, port = await self.endpoint(peer, requested)
        await self.client(port)
        await until(lambda: len(relay.writers) == 2)
        control, control_port = await self.endpoint(peer)
        await self.client(control_port)
        await until(lambda: len(control.writers) == 2)
        # Compare the actual owned sockets with a separate OS socket using the
        # same request; Linux may double it and other kernels may cap it.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as expected:
            expected.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, requested)
            actual = expected.getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF)
        def socket_for(relay, upstream):
            return next(writer.get_extra_info("socket") for writer in relay.writers
                        if (writer.get_extra_info("peername")[1] == relay.port) == upstream)

        self.assertEqual(socket_for(relay, True).getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF), actual)
        self.assertEqual(socket_for(relay, False).getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF),
                         socket_for(control, False).getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF))
        report = relay.report()
        self.assertEqual(report["requestedSendBufferBytes"], requested)
        self.assertEqual(report["minimumObservedUpstreamSendBufferBytes"], actual)
        self.assertEqual(report["maximumObservedUpstreamSendBufferBytes"], actual)
        self.assertEqual(report["socketConfigurationFailures"], 0)

    async def test_explicit_receive_buffer_applies_to_upstream_socket_only(self):
        release = asyncio.Event()

        async def peer(reader, writer):
            await release.wait()

        requested = proxy.CHUNK_BYTES
        relay, port = await self.endpoint(peer, receive_buffer_bytes=requested)
        await self.client(port)
        await until(lambda: len(relay.writers) == 2)
        control, control_port = await self.endpoint(peer)
        await self.client(control_port)
        await until(lambda: len(control.writers) == 2)
        # Compare the actual owned sockets with a separate OS socket using the
        # same request; Linux may double it and other kernels may cap it.
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as expected:
            expected.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, requested)
            actual = expected.getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF)
        def socket_for(relay, upstream):
            return next(writer.get_extra_info("socket") for writer in relay.writers
                        if (writer.get_extra_info("peername")[1] == relay.port) == upstream)

        self.assertEqual(socket_for(relay, True).getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF), actual)
        self.assertEqual(socket_for(relay, False).getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF),
                         socket_for(control, False).getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF))
        report = relay.report()
        self.assertEqual(report["requestedReceiveBufferBytes"], requested)
        self.assertEqual(report["minimumObservedUpstreamReceiveBufferBytes"], actual)
        self.assertEqual(report["maximumObservedUpstreamReceiveBufferBytes"], actual)
        self.assertEqual(report["socketConfigurationFailures"], 0)

    async def test_file_owned_readiness_and_graceful_stop_report(self):
        server = await asyncio.start_server(lambda r, w: w.close(), "127.0.0.1", 0)
        port = server.sockets[0].getsockname()[1]
        server.close()
        await server.wait_closed()
        owner = "012345abcdef"
        with tempfile.TemporaryDirectory(prefix="m2-relay-test-") as directory:
            ready = Path(directory) / f"m2-relay-{owner}.ready"
            stop = ready.with_suffix(".stop")
            args = proxy.configuration([str(port), "198.18.223.2", "--owner", owner,
                                        "--ready-file", str(ready), "--stop-file", str(stop)])
            output = io.StringIO()
            with redirect_stdout(output):
                task = asyncio.create_task(proxy.main(args))
                try:
                    await until(ready.exists)
                    self.assertEqual(ready.read_text("ascii"), owner)
                    stop.write_text(owner, encoding="ascii")
                    await asyncio.wait_for(task, 3)
                finally:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)
            self.assertIn('"activeClients":0', output.getvalue())
            self.assertIn('"ownedWriters":0', output.getvalue())


class ConfigurationTests(unittest.TestCase):
    def test_cleanup_report_validation_is_not_disabled_by_python_optimization(self):
        fixture = Path(__file__).with_name("m2-net-fixture.sh").read_text(encoding="utf-8")
        match = re.search(r"if ! python3 -c '(.*?)'\s+\"\$owned/proxy\.stdout\"", fixture, re.DOTALL)
        self.assertIsNotNone(match, "The shipped cleanup validator must be exercised")
        validator = match[1]
        valid = dict(activeClients=0, ownedWriters=0, socketConfigurationFailures=0, peakClients=16)

        def record(counters):
            return "M2_RELAY " + json.dumps(counters) + "\n"

        cases = [("valid", record(valid), True), ("missing", "", False),
                 ("invalid-json", "M2_RELAY {\n", False),
                 ("duplicate", record(valid) * 2, False),
                 ("negative-peak", record(dict(valid, peakClients=-1)), False)]
        for key, value in (("activeClients", 1), ("ownedWriters", 1),
                           ("socketConfigurationFailures", 1), ("peakClients", 17)):
            cases.append((key, record(dict(valid, **{key: value})), False))
        with tempfile.TemporaryDirectory(prefix="m2-cleanup-test-") as directory:
            report = Path(directory) / "proxy.stdout"
            for name, content, accepted in cases:
                with self.subTest(name=name):
                    report.write_text(content, encoding="ascii")
                    result = subprocess.run([sys.executable, "-O", "-c", validator, str(report)],
                                            capture_output=True, timeout=5, check=False)
                    self.assertEqual(result.returncode == 0, accepted)

    def test_legacy_loopback_and_owned_private_configuration(self):
        legacy = proxy.configuration(["22302", "198.18.223.2"])
        self.assertEqual(legacy.bind, "127.0.0.1")
        self.assertEqual(legacy.send_buffer_bytes, 0)
        args = proxy.configuration(["22302", "198.18.223.2", "--bind", "172.21.1.2", "--owner", "012345abcdef"])
        self.assertEqual(args.bind, "172.21.1.2")

    def test_send_buffer_requires_owner_and_valid_bounded_size(self):
        for value in (proxy.CHUNK_BYTES, proxy.MAX_SEND_BUFFER_BYTES):
            args = proxy.configuration(["22302", "198.18.223.2", "--owner", "012345abcdef",
                                        "--send-buffer-bytes", str(value)])
            self.assertEqual(args.send_buffer_bytes, value)
        for value in (-1, 1, proxy.MAX_SEND_BUFFER_BYTES + 1):
            with self.subTest(value=value), self.assertRaises(ValueError):
                proxy.configuration(["22302", "198.18.223.2", "--owner", "012345abcdef",
                                     "--send-buffer-bytes", str(value)])
        with self.assertRaises(ValueError):
            proxy.configuration(["22302", "198.18.223.2", "--send-buffer-bytes", "65536"])

    def test_receive_buffer_requires_owner_and_valid_bounded_size(self):
        for value in (proxy.CHUNK_BYTES, proxy.MAX_SEND_BUFFER_BYTES):
            args = proxy.configuration(["22302", "198.18.223.2", "--owner", "012345abcdef",
                                        "--receive-buffer-bytes", str(value)])
            self.assertEqual(args.receive_buffer_bytes, value)
        for value in (-1, 1, proxy.MAX_SEND_BUFFER_BYTES + 1):
            with self.subTest(value=value), self.assertRaises(ValueError):
                proxy.configuration(["22302", "198.18.223.2", "--owner", "012345abcdef",
                                     "--receive-buffer-bytes", str(value)])
        with self.assertRaises(ValueError):
            proxy.configuration(["22302", "198.18.223.2", "--receive-buffer-bytes", "65536"])

    def test_invalid_addresses_owners_ports_and_stale_paths_are_rejected(self):
        for arguments in (
            ["80", "198.18.223.2"], ["65536", "198.18.223.2"],
            ["22302", "8.8.8.8"], ["22302", "::1"],
            ["22302", "198.18.223.2", "--bind", "0.0.0.0"],
            ["22302", "198.18.223.2", "--bind", "172.21.1.2"],
            ["22302", "198.18.223.2", "--owner", "incorrect"],
            ["22302", "198.18.223.2", "--ready-file", "unused.ready"],
        ):
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                proxy.configuration(arguments)
        with tempfile.TemporaryDirectory(prefix="m2-relay-test-") as directory:
            ready = Path(directory) / "m2-relay-012345abcdef.ready"
            ready.write_text("stale", encoding="ascii")
            with self.assertRaises(ValueError):
                proxy.configuration(["22302", "198.18.223.2", "--owner", "012345abcdef",
                                     "--ready-file", str(ready), "--stop-file", str(ready.with_suffix(".stop"))])


if __name__ == "__main__":
    unittest.main()
