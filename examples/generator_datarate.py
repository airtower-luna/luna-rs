"""This generator script produces packets calculated for a target data
rate. Parameters are defined at the start of the generate
`function`. See `generator_random.py` for a general description of how
generator modules work.

Usage example:

luna-rs client --py-generator examples/generator_datarate.py -e -O mbps=20

"""
from collections.abc import Iterator

NS_PER_S = 1_000_000_000


def generate(options: dict[str, str]) \
        -> Iterator[tuple[tuple[int, int], int]]:
    # plus IPv6 and UDP headers the default fills 1500 byte MTU
    size = int(options.get('size', '1452'))
    # desired measurement duration in seconds
    duration = int(options.get('duration', '10'))
    # desired datarate in Mbit/s
    mbps = float(options.get('mbps', '1.2'))
    # packets per second for that datarate
    pps = mbps * 1e6 / 8 / size
    # inter send time in ns
    ist = NS_PER_S // int(pps)
    # split into timespec format
    delay = (ist // NS_PER_S, ist % NS_PER_S)
    # packet count to fill duration
    count = int(duration * NS_PER_S) // ist
    for _ in range(count):
        yield (delay, size)
