class Hero:
    def __init__(self, x, y, steps, last_move):
        self.x = x
        self.y = y
        self.steps = steps
        self.last_move = last_move

    def pos(self):
        return (self.x, self.y)

    def at_flag(self, level):
        return self.x == level.flag_pos[0] and self.y == level.flag_pos[1]


class Level:
    def __init__(self, width, height, flag_pos, walls):
        self.width = width
        self.height = height
        self.flag_pos = flag_pos
        self._walls = set(tuple(pos) for pos in walls)

    def is_wall(self, x, y):
        return (x, y) in self._walls


class CommandLog:
    def __init__(self, commands):
        self.list = list(commands)

    @property
    def count(self):
        return len(self.list)


class Events:
    def __init__(self, reached_flag, blocked_moves, errors):
        self.reached_flag = reached_flag
        self.blocked_moves = list(blocked_moves)
        self.errors = list(errors)


class EvalContext:
    def __init__(self, hero, level, commands, events):
        self.hero = hero
        self.level = level
        self.commands = commands
        self.events = events


__all__ = ["Hero", "Level", "CommandLog", "Events", "EvalContext"]
