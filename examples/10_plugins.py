"""Add-ons: pane types that live in their own program (video, web, 3D, your own).

Run:   STAGETUI_PLUGIN_PATH=examples/plugins python examples/10_plugins.py
Try:   the left pane is "pulse", a tiny add-on written in Python
       (examples/plugins/stagetui-plugin-pulse): click it or press space
       the right pane wants the Video add-on: press Enter in it, and stagetui
       asks whether to install it
       click any pane's title to see every add-on in the menu
"""

from stagetui import StageApp, StageBar, StagePane, StageSplit


class AddOns(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("pulse", id="pulse", label="Hello", on_color=self.colored),
                    StagePane("video", id="video"),
                ]
            ),
            StageBar("Click the heart · Enter in the right pane installs Video"),
        ]

    def colored(self, color):
        self.notify(f"The add-on says: {color}")


if __name__ == "__main__":
    AddOns().run()
