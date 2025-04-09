import luna
import pytest
import random
import threading
from contextlib import ExitStack
from decimal import Decimal


def feed(c: luna.Client) -> None:
    r = random.SystemRandom()
    try:
        for _ in range(10):
            c.put(
                (0, 200000000),
                int(r.uniform(luna.MIN_SIZE, c.buffer_size)))
    finally:
        c.close()


def test_client() -> None:
    buf_size = 32
    with ExitStack() as stack:
        server = stack.enter_context(
            luna.Server(bind='::1', port=0, buffer_size=buf_size))
        server_addr = server.bind

        client = luna.Client(server_addr)
        generator_thread = threading.Thread(target=feed, args=(client,))

        # client.run() returns an iterator over logs that'll stop after
        # the client has sent all packets. The client must be closed for
        # it do be done.
        stack.callback(client.close)
        log = client.start()
        generator_thread.start()

        # read all the log lines
        output: list[luna.PacketRecord] = [*log]

    assert server.running is False
    generator_thread.join()
    client.join()

    assert len(output) == 10
    ip, port = server_addr.rsplit(':', maxsplit=1)
    # 1ms should be enough for loopback RTT even on slow systems
    diff = Decimal('0.001')
    for i, record in enumerate(output):
        assert record.source == server_addr
        assert record.sequence == i
        assert record.size == buf_size
        assert isinstance(record.receive_time, Decimal)
        assert isinstance(record.timestamp, Decimal)
        assert record.receive_time - record.timestamp < diff


def test_client_not_connected():
    client = luna.Client('[::1]:7800')
    with pytest.raises(Exception, match=r'^client is not running'):
        client.put((1, 0), 22)


def test_class_name():
    client = luna.Client('[::1]:7800')
    assert repr(client).startswith('<luna.Client object')
