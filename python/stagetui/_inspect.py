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


def plugins(*, refresh: bool = False) -> list[dict[str, Any]]:
    """Add-ons (pane types in their own program): the installed ones, and the
    official ones you can install (video, web, 3D).

    Each is a dict with ``name``, ``title``, ``installed``, ``package`` and ``about``.
    Same as ``stagetui plugins --json``. ``refresh=True`` looks again (after
    installing one while your program runs).
    """
    return json.loads(_core.plugins(refresh))
