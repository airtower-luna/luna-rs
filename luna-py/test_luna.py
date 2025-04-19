import itertools
import luna
import pytest
import random
import threading
from contextlib import ExitStack
from decimal import Decimal


def feed(c: luna.Client, packets: int, sizes: list[int]) -> None:
    r = random.SystemRandom()
    for _ in range(packets):
        size = r.randint(luna.MIN_SIZE, c.buffer_size)
        c.put((0, 2000000), size)
        sizes.append(size)


def client_timeout(
        c: luna.Client, event: threading.Event, timeout: float) -> None:
    """Close the client after either the event is set, or the timeout
    expires. This provides a fallback in case the client isn't closed
    after all expected packets have been received.

    """
    try:
        event.wait(timeout)
    finally:
        c.close()


def test_full() -> None:
    buf_size = 1500
    packets = 10
    with ExitStack() as stack:
        server = stack.enter_context(
            luna.Server(bind='::1', port=0, buffer_size=buf_size))
        server_addr = server.bind
        server_log: list[luna.PacketRecord] = list()
        server_log_thread = threading.Thread(
            target=lambda: server_log.extend(
                itertools.islice(server, packets)))
        server_log_thread.start()

        client = luna.Client(server_addr)
        sizes: list[int] = list()
        generator_thread = threading.Thread(
            target=feed, args=(client, packets, sizes))

        # The event stops the timeout thread early, so it can be
        # joined after leaving the context (after all expected packets
        # have been received).
        done_event = threading.Event()
        stack.callback(done_event.set)
        timeout_thread = threading.Thread(
            target=client_timeout, args=(client, done_event, 3.0))

        # client.start() or entering its context returns an iterator
        # over logs that'll stop after the client has sent all
        # packets. The client must be closed (or the context left) for
        # the iterator to end.
        stack.enter_context(client)
        generator_thread.start()
        timeout_thread.start()

        # read the expected number of log lines
        client_log = [*itertools.islice(client, packets)]

    assert server.running is False
    assert client.running is False
    generator_thread.join()
    timeout_thread.join()
    server_log_thread.join()

    assert len(client_log) == 10
    ip, port = server_addr.rsplit(':', maxsplit=1)
    # 100ms should be enough for loopback RTT even on slow systems
    diff = Decimal('0.100')
    for i, record in enumerate(client_log):
        assert record.source == server_addr
        assert record.sequence == i
        assert record.size == sizes[i]
        assert isinstance(record.receive_time, Decimal)
        assert isinstance(record.timestamp, Decimal)
        assert record.receive_time - record.timestamp < diff

    assert len(server_log) == 10
    # 50ms should be enough for loopback one-way even on slow systems
    diff = Decimal('0.050')
    for i, record in enumerate(server_log):
        assert record.source.startswith('[::1]:')
        assert record.sequence == i
        assert record.size == sizes[i]
        assert isinstance(record.receive_time, Decimal)
        assert isinstance(record.timestamp, Decimal)
        assert record.receive_time - record.timestamp < diff

    assert repr(client_log[0]).startswith(
        '<luna.PacketRecord: ReceivedPacket {')


def test_client_not_connected():
    client = luna.Client('[::1]:7800')
    with pytest.raises(Exception, match=r'^client is not running'):
        client.put((1, 0), 22)


def test_class_name():
    client = luna.Client('[::1]:7800')
    assert repr(client).startswith('<luna.Client object')


def test_server_double_join():
    with luna.Server(bind='::1', port=0, buffer_size=luna.MIN_SIZE) as server:
        pass
    server.join()


def test_client_double_join():
    with luna.Server(bind='::1', port=0, buffer_size=luna.MIN_SIZE) as server:
        client = luna.Client(server.bind)
        with client:
            client.close()
    client.join()
