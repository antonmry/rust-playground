from typing import Iterable, Sequence, Tuple, Optional, List

class Hero:
    x: int
    y: int
    steps: int
    last_move: Optional[str]
    def __init__(self, x: int, y: int, steps: int, last_move: Optional[str]) -> None: ...
    def pos(self) -> Tuple[int, int]: ...
    def at_flag(self, level: "Level") -> bool: ...

class Level:
    width: int
    height: int
    flag_pos: Tuple[int, int]
    def __init__(
        self,
        width: int,
        height: int,
        flag_pos: Tuple[int, int],
        walls: Iterable[Tuple[int, int]],
    ) -> None: ...
    def is_wall(self, x: int, y: int) -> bool: ...

class CommandLog:
    list: List[str]
    @property
    def count(self) -> int: ...
    def __init__(self, commands: Sequence[str]) -> None: ...

class Events:
    reached_flag: bool
    blocked_moves: List[str]
    errors: List[str]
    def __init__(
        self, reached_flag: bool, blocked_moves: Sequence[str], errors: Sequence[str]
    ) -> None: ...

class EvalContext:
    hero: Hero
    level: Level
    commands: CommandLog
    events: Events
    def __init__(self, hero: Hero, level: Level, commands: CommandLog, events: Events) -> None: ...

__all__: Sequence[str]
