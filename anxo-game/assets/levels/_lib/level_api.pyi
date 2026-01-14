from typing import TypedDict, List, Union

class Point(TypedDict):
    x: int
    y: int

class EvalContext(TypedDict):
    hero: Point
    flag: Point
    steps: int
    commands: List[str]
    reached_flag: bool

def evaluate(context: EvalContext) -> Union[bool, str]: ...
