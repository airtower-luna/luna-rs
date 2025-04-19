# LUNA in Rust

Luna-rs is a re-implementation of [LUNA (Lightweight Universal Network
Analyzer)](https://github.com/airtower-luna/luna) in Rust. The
original tool was my graduation thesis project, the re-implementation
is primarily Rust practice. The packet format is compatible, the
output format is different (still tab separated values, but different
columns, now unified between client and server).

The examples below use `cargo run` to run the binary from the build
directory, the installed binary will be called `luna-rs`.

**Note:** This is a learning project, I make no promises on API
stability or anything else.

## Trying it

In separate terminals, run:

```
$ cargo run -- server
$ cargo run -- client -e
```

See `-h` output for options. "Generators" set how packets are sent,
see below for options.

The process will try to lock its memory in RAM (no swapping) and
elevate the main thread to realtime priority for best timing behavior,
doing so requires the `CAP_SYS_NICE` and `CAP_IPC_LOCK` capabilities
(the latter might not be necessary depending on rlimits on your
system). It will still run without those capabilities, just with error
messages during start.


## Built-in generators

Two built-in generators are defined in
[`src/generator.rs`](./src/generator.rs): "default" and "vary". The
"default" generator sends packets with fixed (configurable) size and
interval. The "vary" generator doubles the size with every packet
until the size exceeds the maximum, then halves it with every packet
until it meets the minimum size, and repeats. Options are set using
the `-O` or `--generator-option` command line options, e.g. to send
512 byte packets using the "default" generator:

```sh
$ cargo run -- client -e -g default -O size=512
```

### Shared options

* One of the following options may be given to set the interval at
  which packets are sent:
    * `interval`: packet interval in seconds, with up to 9 decimal places
    * `msec`: packet interval in milliseconds (integer)
    * `usec`: packet interval in microseconds (integer)
    * `nsec`: packet interval in nanoseconds (integer)
* `count` sets the number of packets to send

### "Default" generator options

* `size`: size of packets to send, in bytes of UDP payload

### "Vary" generator options

* `max-size`: maximum size of packets to send, in bytes of UDP payload


## Python bindings :snake:

The [`luna-py/` directory](./luna-py/) contains Python bindings using
PyO3. You can use [Maturin](https://www.maturin.rs/) to build a wheel
package (`maturin build`), or run
[Nox](https://nox.thea.codes/en/stable/) to build and test
(`nox`). [See the test](./luna-py/test_luna.py) for a usage example.

## Python generators

Instead of integrating LUNA into your Python program, you can also
write only the generator in Python, and have the client call it
instead of one of the built-in generators. See
[`examples/generator_random.py`](./examples/generator_random.py) for
an annotated example. To use a Python generator, set it on the command
line, for example:

```sh
$ cargo run -- client -e --py-generator examples/generator_random.py -O count=100
```

Python generators can accept options the same way the built-in
generators do (see above), all generator options passed on the command
line will be passed to the `generate` function as a `dict[str, str]`.
