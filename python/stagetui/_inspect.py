from __future__ import annotations

import json
from typing import Any

from . import _core


def inspect(*, headless: bool = False, probe: bool = True) -> dict[str, Any]:
    """Find out what this terminal can do: images, keyboard, colors, size, Arabic support…

    Same information as ``stagetui inspect`` on the command line.
    """
    return json.loads(_core.inspect(headless=headless, probe=probe))


def fonts() -> dict[str, Any]:
    """Which writing systems (Latin, Arabic, Chinese, …) have fonts available for pictures."""
    return json.loads(_core.fonts())


def sessions() -> list[dict[str, Any]]:
    """Apps running right now that another terminal can attach to.

    Each one is a dict with ``name``, ``pid``, ``app``, ``cwd`` and ``started``.
    Same information as ``stagetui sessions --json``; open one with
    ``stagetui attach <name>``.
    """
    return json.loads(_core.sessions())
