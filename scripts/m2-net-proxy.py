"""Bounded fixture relay. Only counts are reported; traffic is never logged."""
import argparse
import asyncio
import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import socket


CHUNK_BYTES = 65536
MAX_CLIENTS = 16
MAX_SEND_BUFFER_BYTES = 4 * 1024 * 1024
PRIVATE_NETWORKS = tuple(map(ipaddress.ip_network, (
    "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16",
)))


def private_address(value):
    address = ipaddress.IPv4Address(value)
    if not any(address in network for network in PRIVATE_NETWORKS):
        raise ValueError("Expected an assigned private IPv4 address")
    return str(address)


def configuration(arguments=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("port", type=int)
    parser.add_argument("address")
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--owner")
    parser.add_argument("--ready-file", type=Path)
    parser.add_argument("--stop-file", type=Path)
    parser.add_argument("--send-buffer-bytes", type=int, default=0,
                        help="Owned relay upstream socket only; zero keeps OS defaults")
    parser.add_argument("--receive-buffer-bytes", type=int, default=0,
                        help="Owned relay upstream receive socket only; zero keeps OS defaults")
    args = parser.parse_args(arguments)
    if not 1024 <= args.port <= 65535:
        raise ValueError("Invalid fixture port")
    if args.address != "198.18.223.2":
        args.address = private_address(args.address)
    if args.bind != "127.0.0.1":
        args.bind = private_address(args.bind)
    if args.owner is not None and re.fullmatch(r"[0-9a-f]{12}", args.owner) is None:
        raise ValueError("Invalid fixture owner")
    if (args.send_buffer_bytes != 0
            and not CHUNK_BYTES <= args.send_buffer_bytes <= MAX_SEND_BUFFER_BYTES):
        raise ValueError("Invalid fixture socket send buffer")
    if args.receive_buffer_bytes != 0 and not CHUNK_BYTES <= args.receive_buffer_bytes <= MAX_SEND_BUFFER_BYTES:
        raise ValueError("Invalid fixture socket receive buffer")
    if (args.send_buffer_bytes or args.receive_buffer_bytes) and args.owner is None:
        raise ValueError("Socket configuration requires fixture ownership")
    if args.bind != "127.0.0.1" and args.owner is None:
        raise ValueError("Private listeners require fixture ownership")
    if bool(args.ready_file) != bool(args.stop_file):
        raise ValueError("Ready and stop paths must be supplied together")
    if args.ready_file:
        if args.owner is None:
            raise ValueError("File-controlled relays require fixture ownership")
        args.ready_file = args.ready_file.resolve()
        args.stop_file = args.stop_file.resolve()
        if (args.ready_file.parent != args.stop_file.parent
                or args.ready_file.name != f"m2-relay-{args.owner}.ready"
                or args.stop_file.name != f"m2-relay-{args.owner}.stop"
                or args.ready_file.exists() or args.stop_file.exists()):
            raise ValueError("Invalid or previously used owned relay paths")
    return args


class Relay:
    def __init__(self, address, port, send_buffer_bytes=0, receive_buffer_bytes=0):
        self.address = address
        self.port = port
        self.server = None
        self.tasks = set()
        self.writers = set()
        self.accepted = 0
        self.rejected = 0
        self.peak_clients = 0
        self.forwarded_bytes = 0
        self.send_buffer_bytes = send_buffer_bytes
        self.receive_buffer_bytes = receive_buffer_bytes
        self.minimum_upstream_receive_buffer_bytes = 0
        self.maximum_upstream_receive_buffer_bytes = 0
        self.socket_configuration_failures = 0
        self.minimum_send_buffer_bytes = 0
        self.maximum_send_buffer_bytes = 0
        self.minimum_upstream_send_buffer_bytes = 0
        self.maximum_upstream_send_buffer_bytes = 0

    def configure_socket(self, writer, upstream=False):
        connection = writer.get_extra_info("socket")
        if upstream and self.send_buffer_bytes:
            # Linux accounts twice the requested size. This changes only this
            # owned socket; it does not change host or namespace sysctls.
            connection.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, self.send_buffer_bytes)
        if upstream and self.receive_buffer_bytes:
            connection.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, self.receive_buffer_bytes)
        if upstream:
            received = connection.getsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF)
            self.minimum_upstream_receive_buffer_bytes = min(self.minimum_upstream_receive_buffer_bytes or received, received)
            self.maximum_upstream_receive_buffer_bytes = max(self.maximum_upstream_receive_buffer_bytes, received)
        actual = connection.getsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF)
        self.minimum_send_buffer_bytes = min(self.minimum_send_buffer_bytes or actual, actual)
        self.maximum_send_buffer_bytes = max(self.maximum_send_buffer_bytes, actual)
        if upstream:
            self.minimum_upstream_send_buffer_bytes = min(self.minimum_upstream_send_buffer_bytes or actual, actual)
            self.maximum_upstream_send_buffer_bytes = max(self.maximum_upstream_send_buffer_bytes, actual)

    async def start(self, bind, port):
        self.server = await asyncio.start_server(self.accept, bind, port, limit=CHUNK_BYTES)
        return self.server.sockets[0].getsockname()[1]

    def accept(self, reader, writer):
        if len(self.tasks) >= MAX_CLIENTS:
            self.rejected += 1
            writer.close()
            return
        self.accepted += 1
        try:
            self.configure_socket(writer)
        except OSError:
            self.socket_configuration_failures += 1
            writer.close()
            return
        self.writers.add(writer)
        task = asyncio.create_task(self.connect(reader, writer))
        self.tasks.add(task)
        self.peak_clients = max(self.peak_clients, len(self.tasks))
        task.add_done_callback(self.tasks.discard)

    async def pump(self, reader, writer):
        while data := await reader.read(CHUNK_BYTES):
            writer.write(data)
            await writer.drain()
            self.forwarded_bytes += len(data)
        # Preserve half-close: a peer may reply after receiving the final byte.
        if writer.can_write_eof():
            writer.write_eof()
            await writer.drain()

    async def connect(self, reader, writer):
        owned = [writer]
        pumps = []
        try:
            peer_reader, peer_writer = await asyncio.wait_for(
                asyncio.open_connection(self.address, self.port, limit=CHUNK_BYTES), 5,
            )
            owned.append(peer_writer)
            self.writers.add(peer_writer)
            try:
                self.configure_socket(peer_writer, upstream=True)
            except OSError:
                self.socket_configuration_failures += 1
                raise
            pumps = [asyncio.create_task(self.pump(reader, peer_writer)),
                     asyncio.create_task(self.pump(peer_reader, writer))]
            await asyncio.gather(*pumps)
        except (ConnectionError, OSError, asyncio.TimeoutError):
            pass
        finally:
            for pump in pumps:
                pump.cancel()
            await asyncio.gather(*pumps, return_exceptions=True)
            for stream in owned:
                stream.close()
            for stream in owned:
                try:
                    await asyncio.wait_for(stream.wait_closed(), 1)
                except (ConnectionError, OSError, asyncio.TimeoutError):
                    stream.transport.abort()
                self.writers.discard(stream)

    async def close(self):
        if self.server is not None:
            self.server.close()
        for writer in tuple(self.writers):
            # Shutdown cancels forwarding, so pending peer output must not keep
            # a Windows proactor transport alive while its reader is paused.
            writer.transport.abort()
        pending = tuple(self.tasks)
        for task in pending:
            task.cancel()
        await asyncio.gather(*pending, return_exceptions=True)
        self.tasks.difference_update(pending)
        self.writers.clear()
        # Close and join accepted transports before waiting for the listening
        # server to finish; a peer may have stopped consuming output.
        if self.server is not None:
            await self.server.wait_closed()

    def report(self):
        return dict(accepted=self.accepted, rejected=self.rejected,
                    peakClients=self.peak_clients, activeClients=len(self.tasks),
                    ownedWriters=len(self.writers), forwardedBytes=self.forwarded_bytes,
                    chunkBytes=CHUNK_BYTES, maximumClients=MAX_CLIENTS,
                    requestedSendBufferBytes=self.send_buffer_bytes,
                    requestedReceiveBufferBytes=self.receive_buffer_bytes,
                    minimumObservedUpstreamReceiveBufferBytes=self.minimum_upstream_receive_buffer_bytes,
                    maximumObservedUpstreamReceiveBufferBytes=self.maximum_upstream_receive_buffer_bytes,
                    minimumObservedSendBufferBytes=self.minimum_send_buffer_bytes,
                    maximumObservedSendBufferBytes=self.maximum_send_buffer_bytes,
                    minimumObservedUpstreamSendBufferBytes=self.minimum_upstream_send_buffer_bytes,
                    maximumObservedUpstreamSendBufferBytes=self.maximum_upstream_send_buffer_bytes,
                    socketConfigurationFailures=self.socket_configuration_failures)


async def main(args):
    stopped = asyncio.Event()
    if os.name != "nt":
        asyncio.get_running_loop().add_signal_handler(signal.SIGTERM, stopped.set)
    relay = Relay(args.address, args.port, args.send_buffer_bytes, args.receive_buffer_bytes)
    await relay.start(args.bind, args.port)

    async def watch_stop():
        while not stopped.is_set():
            if args.stop_file.exists():
                if args.stop_file.read_text(encoding="ascii").strip() != args.owner:
                    raise ValueError("Relay stop owner differs")
                stopped.set()
                return
            await asyncio.sleep(0.1)

    watcher = None
    try:
        if args.ready_file:
            args.ready_file.write_text(args.owner, encoding="ascii")
            watcher = asyncio.create_task(watch_stop())
            stopped_wait = asyncio.create_task(stopped.wait())
            try:
                done, _ = await asyncio.wait((watcher, stopped_wait), return_when=asyncio.FIRST_COMPLETED)
                for task in done:
                    task.result()
            finally:
                stopped_wait.cancel()
                await asyncio.gather(stopped_wait, return_exceptions=True)
        else:
            await stopped.wait()
    finally:
        if watcher is not None:
            watcher.cancel()
            await asyncio.gather(watcher, return_exceptions=True)
        await relay.close()
        print("M2_RELAY " + json.dumps(relay.report(), separators=(",", ":")), flush=True)


if __name__ == "__main__":
    asyncio.run(main(configuration()))
