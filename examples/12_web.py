"""A real web page in a pane — from the Chrome or Chromium on your computer.

Run:   python examples/12_web.py [address]
Needs: the web add-on: pip install stagetui-web
Try:   click links, scroll, type in forms (any language)
       Ctrl+L type an address · Alt+← → back / forward · F5 reload
       click ↗ in the pane's title to open the page in its own window
"""

import sys

from stagetui import StageApp, StageBar, StagePane, StageSplit

URL = sys.argv[1] if len(sys.argv) > 1 else "https://example.com"

NOTES = """# Browsing

The left pane is a real page from your
browser — links, forms, CSS, JavaScript.

| Key | Does |
|---|---|
| Ctrl+L | type an address |
| Alt+← → | back / forward |
| F5 | reload |

Click the page first, then type.
"""


class Browse(StageApp):
    theme = "aurora-dark"

    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("web", id="web", url=URL, on_title=self.titled),
                    StagePane("markdown", text=NOTES),
                ],
                ratio=[3, 1],
                grow=1,
            ),
            StageBar("Ctrl+L address · Alt+← → back/forward · F5 reload · Ctrl+Space q quit"),
        ]

    def titled(self, title):
        if title:
            self.notify(f"Opened: {title}")


if __name__ == "__main__":
    Browse().run()
