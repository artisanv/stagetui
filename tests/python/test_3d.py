"""The 3D add-on, end to end: models in a pane, turned with the mouse and keys.

Runs when the add-on (`stagetui-plugin-3d`) is available — in CI the "3D add-on"
job builds it and points STAGETUI_PLUGIN_PATH at it.
"""

import base64
import json
import struct
import time
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import StageApp, StagePane

# A name in several scripts: paths with any letters must work.
NAME = "مَرْحَبًا Hello 日本語 नमस्ते"


def cube_obj(path: Path) -> Path:
    path.write_text(
        "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n"
        "f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 3 4 8 7\nf 2 3 7 6\nf 1 5 8 4\n",
        encoding="utf-8",
    )
    return path


def pyramid_stl(path: Path) -> Path:
    """A binary STL: four triangles."""
    p = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1)]
    faces = [(0, 2, 1), (0, 1, 3), (0, 3, 2), (1, 2, 3)]
    data = b"binary stl".ljust(80, b"\0") + struct.pack("<I", len(faces))
    for f in faces:
        data += struct.pack("<3f", 0, 0, 0)
        for i in f:
            data += struct.pack("<3f", *p[i])
        data += b"\0\0"
    path.write_bytes(data)
    return path


def triangle_gltf(path: Path) -> Path:
    """A .gltf with its data inside (base64): one red triangle."""
    raw = struct.pack("<9f", 0, 0, 0, 1, 0, 0, 0, 1, 0) + struct.pack("<3H", 0, 1, 2) + b"\0\0"
    doc = {
        "asset": {"version": "2.0"},
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [1, 0, 0, 1]}}],
        "buffers": [
            {
                "byteLength": len(raw),
                "uri": "data:application/octet-stream;base64," + base64.b64encode(raw).decode(),
            }
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 6},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"},
        ],
    }
    path.write_text(json.dumps(doc), encoding="utf-8")
    return path


@pytest.fixture
def models(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> dict[str, Path]:
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    if not any(p["name"] == "3d" and p["installed"] for p in ts.plugins(refresh=True)):
        pytest.skip("the 3D add-on isn't built here")
    return {
        "obj": cube_obj(tmp_path / f"{NAME}.obj"),
        "stl": pyramid_stl(tmp_path / "pyramid.stl"),
        "gltf": triangle_gltf(tmp_path / "triangle.gltf"),
    }


def until(d, check, seconds: float = 20.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        d.wait(0.05)
        try:
            if check():
                return True
        except ts.StageError:
            pass
        time.sleep(0.05)
    return False


def test_turn_zoom_and_switch_models(models: dict[str, Path], tmp_path: Path) -> None:
    events: list[dict] = []

    class Viewer(StageApp):
        def compose(self):
            return [
                StagePane(
                    "3d", id="m", path=str(models["obj"]), spin=False, on_loaded=events.append
                )
            ]

    with Viewer().drive((80, 24)) as d:
        node = lambda: d.query("#m")["children"][0]
        model = d.app.query("#m").content
        assert until(d, lambda: node()["frames"] > 0), "a picture arrived"
        assert until(d, lambda: "12 triangles" in (node()["status"] or ""))
        assert until(d, lambda: bool(events))
        assert events[0]["triangles"] == 12 and events[0]["path"].endswith(f"{NAME}.obj")
        state = model.call("state")
        assert state["format"] == "OBJ" and state["spin"] is False

        # Drag to turn: a third of the pane's width is a third of a turn.
        b = node()["bounds"]
        yaw = model.call("state")["yaw"]
        d.drag((b["x"] + 10, b["y"] + 5), (b["x"] + 10 + b["w"] // 3, b["y"] + 5))
        assert until(d, lambda: abs(model.call("state")["yaw"] - yaw) > 60)
        model.call("view", {"yaw": 45, "pitch": 10})
        assert abs(model.call("state")["yaw"] - 45) < 0.01

        # The wheel zooms.
        far = model.call("state")["distance"]
        d.scroll(-3, target="#m")
        assert until(d, lambda: model.call("state")["distance"] < far)

        # Keys (the pane has focus after the click).
        d.press("w")
        assert until(d, lambda: model.call("state")["wireframe"] is True)
        d.press("space")
        assert until(d, lambda: model.call("state")["spin"] is True)
        d.press("r")
        assert until(d, lambda: abs(model.call("state")["distance"] - far) < 1e-3)

        # Save what it shows.
        png = tmp_path / "view.png"
        assert model.call("save", str(png)) == str(png)
        assert png.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"

        # Other formats.
        assert model.call("load", str(models["stl"]))["triangles"] == 4
        assert model.call("state")["format"] == "STL"
        assert model.call("load", str(models["gltf"]))["format"] == "glTF"
        assert until(d, lambda: len(events) == 3)

        with pytest.raises(ts.StageError, match="not found"):
            model.call("load", str(tmp_path / "missing.glb"))
        with pytest.raises(ts.StageError, match="no method"):
            model.call("explode")


def test_problems_are_explained(models: dict[str, Path], tmp_path: Path) -> None:
    odd = tmp_path / "model.fbx"
    odd.write_bytes(b"x")

    class Viewer(StageApp):
        def compose(self):
            return [StagePane("3d", id="a", path=str(odd)), StagePane("3d", id="b")]

    with Viewer().drive((100, 24)) as d:
        # The pane says what went wrong (long temporary paths wrap on screen, so read its data).
        error = lambda pane: d.query(pane)["children"][0]["error"] or ""
        assert until(d, lambda: "export it as one of those" in error("#a"))
        assert until(d, lambda: 'path="model.glb"' in error("#b"))
        assert "could not start" in d.snapshot().text
