import luna_py  # type: ignore
import random
import threading


def feed(c):
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
    # WARNING: There's no way to *stop* the server yet, it will run
    # until the process terminates.
    server: str = luna_py.spawn_server(
        bind='::1', port=0, buffer_size=buf_size)
    client = luna_py.Client(server)
    generator_thread = threading.Thread(target=feed, args=(client,))

    # client.run() returns an iterator over logs that'll stop after
    # the client has sent all packets. The client must be closed for
    # it do be done.
    log = client.run()
    generator_thread.start()

    # read all the log lines
    output: list[str] = [*log]
    client.join()
    generator_thread.join()
    assert len(output) == 10
    ip, port = server.rsplit(':', maxsplit=1)
    for i, line in enumerate(output):
        fields = line.split('\t')
        assert fields[1] == ip.strip('[]')
        assert fields[2] == port
        assert int(fields[3]) == i
        assert int(fields[5]) == buf_size
