class _Hero:
    def move_left(self) -> None: ...
    def move_right(self) -> None: ...
    def pick(self, obj: object) -> None: ...
    def open(self, obj: object) -> None: ...

class _Key:
    pass

class _Padlock:
    pass

hero: _Hero
key: _Key
padlock: _Padlock

__all__ = ["hero", "key", "padlock"]
