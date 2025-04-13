"""This is an example of a generator script for use with
LUNA. Generator scripts must define a `generate` function, and may use
the MIN_SIZE variable that is injected before calling the function.

Example command (luna-rs must be built with "python" feature, enabled
by default):

luna-rs client --py-generator examples/generator_random.py -e

"""
import random
from collections.abc import Iterator

# correct value is injected before generate is called, exists in the
# script only for linters
MIN_SIZE = 0


def generate() -> Iterator[tuple[tuple[int, int], int]]:
    """This function returns an Iterator which yields tuples that
    describe packets to send. The inner tuple sets the send time
    relative to the previous packet in seconds and nanoseconds, the
    final `int` the size of the packet in bytes.

    """
    r = random.SystemRandom()
    for _ in range(200):
        size = r.randint(MIN_SIZE, 512)
        yield ((0, 1_000_000), size)
