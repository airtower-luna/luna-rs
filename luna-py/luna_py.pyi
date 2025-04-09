MIN_SIZE: int


class LogIter:
    def __iter__(self) -> 'LogIter': ...
    def __next__(self) -> str: ...


class Server:
    buffer_size: int

    def __init__(self, bind: str, port: int = 7800, buffer_size: int = 1500) \
            -> None:
        ...

    def start(self) -> str: ...

    def stop(self) -> None: ...


class Client:
    buffer_size: int
    echo: bool
    running: bool

    def __init__(
            self, server: str, buffer_size: int = 1500, echo: bool = True) \
            -> None:
        ...

    def start(self) -> LogIter: ...

    def put(self, delay: tuple[int, int], size: int) -> None: ...

    def close(self) -> None: ...

    def join(self) -> None: ...
