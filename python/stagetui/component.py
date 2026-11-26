from __future__ import annotations

import functools
from collections.abc import Callable, Sequence
from typing import ParamSpec

from .widgets import Stage, _Widget

P = ParamSpec("P")


def component(fn: Callable[P, _Widget | Sequence[_Widget]]) -> Callable[P, _Widget]:
    """Turn a function that builds widgets into a reusable widget.

    The function can return one widget, or a list of widgets (they are stacked
    in a :class:`Stage`). Use it like any other widget.

    Example::

        @ts.component
        def MetricTile(label: str, value: str, accent: str = "mint"):
            return StageCard(f"{value}\\n{label}", accent=accent, animate="pop")

        class Dashboard(StageApp):
            def compose(self):
                return [Stage([MetricTile("Users", "1,204"), MetricTile("Errors", "3", "rose")],
                              direction="horizontal")]
    """

    @functools.wraps(fn)
    def wrapper(*args: P.args, **kwargs: P.kwargs) -> _Widget:
        result = fn(*args, **kwargs)
        if isinstance(result, _Widget):
            return result
        if isinstance(result, Sequence) and not isinstance(result, str):
            return Stage(list(result))
        raise TypeError(
            f"@component {fn.__name__}() must return a widget or a list of widgets, "
            f"got {type(result).__name__}"
        )

    wrapper.__stagetui_component__ = True  # type: ignore[attr-defined]
    return wrapper
