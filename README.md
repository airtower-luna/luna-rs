# LUNA in Rust

Luna-rs is a re-implementation of [LUNA (Lightweight Universal Network
Analyzer)](https://github.com/airtower-luna/luna) in Rust. The
original tool was my graduation thesis project, the re-implementation
is primarily Rust practice. The packet format is compatible, the
output format is different (still tab separated values, but different
columns, now unified between client and server).

**Note:** This is a learning project, I make no promises on API
stability or anything else.

## Trying it

In separate terminals, run:

```
$ cargo run -- server
$ cargo run -- client -e
```

See `-h` output for options.

The process will try to lock its memory in RAM (no swapping) and
elevate the main thread to realtime priority for best timing behavior,
doing so requires the `CAP_SYS_NICE` and `CAP_IPC_LOCK` capabilities
(the latter might not be necessary depending on rlimits on your
system). It will still run without those capabilities, just with error
messages during start.

## Python bindings

The [`luna-py/` directory](./luna-py/) contains Python bindings using
PyO3. You can use [Maturin](https://www.maturin.rs/) to build a wheel
package (`maturin build`), or run
[Nox](https://nox.thea.codes/en/stable/) to build and test
(`nox`). [See the test](./luna-py/test_luna.py) for a usage example.
