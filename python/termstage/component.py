from __future__ import annotations

import functools
from collections.abc import Callable
from typing import ParamSpec, TypeVar

P = ParamSpec("P")
W = TypeVar("W")


def component(fn: Callable[P, W]) -> Callable[P, W]:
    """Mark a function that returns widgets as a reusable termstage component.

    Example::

        @ts.component
        def MetricTile(label: str, value: str, accent: str = "mint"):
            return StageCard(f"{value}\\n{label}", accent=accent, animate="pop")
    """

    @functools.wraps(fn)
    def wrapper(*args: P.args, **kwargs: P.kwargs) -> W:
        return fn(*args, **kwargs)

    wrapper.__termstage_component__ = True  # type: ignore[attr-defined]
    return wrapper
