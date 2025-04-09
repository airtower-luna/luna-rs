import luna_py
import random
import threading
from contextlib import ExitStack


def feed(c: luna_py.Client):
    r = random.SystemRandom()
    try:
        for _ in range(10):
            c.put(
                (0, 200000000),
                int(r.uniform(luna_py.MIN_SIZE, c.buffer_size)))
    finally:
        c.close()


def test_client() -> None:
    buf_size = 32
    with ExitStack() as stack:
        server = luna_py.Server(bind='::1', port=0, buffer_size=buf_size)
        stack.callback(server.stop)
        server_addr = server.start()

        client = luna_py.Client(server_addr)
        generator_thread = threading.Thread(target=feed, args=(client,))

        # client.run() returns an iterator over logs that'll stop after
        # the client has sent all packets. The client must be closed for
        # it do be done.
        stack.callback(client.close)
        log = client.start()
        generator_thread.start()

        # read all the log lines
        output: list[str] = [*log]

    generator_thread.join()
    client.join()

    assert len(output) == 10
    ip, port = server_addr.rsplit(':', maxsplit=1)
    for i, line in enumerate(output):
        fields = line.split('\t')
        assert fields[1] == ip.strip('[]')
        assert fields[2] == port
        assert int(fields[3]) == i
        assert int(fields[5]) == buf_size
