# stagetui-3d

The 3D add-on for [stagetui](https://github.com/artisanv/stagetui): look at 3D
models in any pane and turn them with the mouse. It reads **glTF** (`.glb`,
`.gltf`), **OBJ** and **STL** files — with their colors and pictures — and draws
them on your computer's processor, so no graphics card or driver is needed.

```bash
pip install stagetui-3d      # also installs stagetui itself
```

```python
from stagetui import StageApp, StagePane


class Look(StageApp):
    def compose(self):
        return [StagePane("3d", path="robot.glb")]


Look().run()
```

Drag to turn it, right-drag to move it, and the mouse wheel zooms. With the pane
focused: **Space** spins or stops, **W** shows the wires, **G** the floor grid,
**R** starts over.

Models look best in a terminal with real pictures (Ghostty, Kitty, WezTerm, foot,
iTerm2, Windows Terminal); in other terminals stagetui can open the pane in one of
those for you.

See the [add-ons guide](https://github.com/artisanv/stagetui/blob/main/docs/plugins.md#3d).
