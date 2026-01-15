from level_api import EvalContext


def evaluate(context: EvalContext) -> bool | str:
    if not context.events.key_collected:
        return "Pick up the key first."
    if not context.events.lock_unlocked:
        return "Unlock the padlock before reaching the flag."
    if context.hero.at_flag(context.level):
        return True
    if context.events.blocked_moves:
        return "You can't move there."
    return "Reach the flag to complete this level."
