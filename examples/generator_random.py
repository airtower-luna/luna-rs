"""This is an example of a generator script for use with
LUNA. Generator scripts must define a `generate` function, and may use
the MIN_SIZE variable that is injected before calling the function.

Example command (must be built with "python" feature):

luna-rs client --py-generator examples/generator_random.py -e
"""
import random

# correct value is injected before generate is called, exists in the
# script only for linters
MIN_SIZE = 0


def generate():
    r = random.SystemRandom()
    for _ in range(10):
        size = r.randint(MIN_SIZE, 512)
        yield ((0, 1000000), size)
