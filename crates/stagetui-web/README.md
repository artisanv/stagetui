# stagetui-web

The web add-on for [stagetui](https://github.com/artisanv/stagetui): a real web
page in any pane, shown by the Chrome, Chromium, Edge or Brave already on your
computer (nothing is downloaded).

```bash
pip install stagetui-web      # also installs stagetui itself
```

```python
from stagetui import StageApp, StagePane


class Browse(StageApp):
    def compose(self):
        return [StagePane("web", url="https://example.com")]


Browse().run()
```

Click, scroll and type right in the page — in any language. **Ctrl+L** types an
address, **Alt+← / Alt+→** go back and forward, **F5** reloads.

Pages look best in a terminal with real pictures (Ghostty, Kitty, WezTerm, foot,
iTerm2, Windows Terminal); in other terminals stagetui can open the page in one of
those for you.

See the [add-ons guide](https://github.com/artisanv/stagetui/blob/main/docs/plugins.md).
