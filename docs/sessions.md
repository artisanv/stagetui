# Sessions: keep an app running, come back later

Some apps should keep working when you walk away: an AI agent fixing a bug, a
long build, a chat that is still answering. With stagetui you can **start an app
in the background**, close the terminal, and **attach** to it again later —
from the same terminal or a new one. Nothing stops in between: shells keep
running, chats keep streaming, timers keep ticking.

If you know `tmux` or `screen`, it is the same idea, built into your app.

`examples/09_detach.py` is a pretend agent to try it with.

## Try it in one minute

```bash
stagetui start examples/09_detach.py
```

The app opens in your terminal. Now press **Ctrl+Space**, then **d**
("detach"). You get your terminal back — but the app is still running:

```bash
stagetui sessions
```

```
NAME           PID  STARTED       APP
night-shift  48213  2 min ago     examples/09_detach.py

Open one with: stagetui attach <name>
```

Come back whenever you like:

```bash
stagetui attach night-shift
```

Everything it did while you were away is on screen.

## Leaving: detach or quit?

When you press **q** (or **Ctrl+C**) in an attached window, stagetui asks first:

```
╭─ Leave "night-shift"? ─────────────────────────╮
│                                                │
│  Detach: it keeps running, come back later.    │
│  Quit: the app stops for every window.         │
│                                                │
│        Detach      Quit app      Cancel        │
│                                                │
│  d detach · q quit · Esc cancel · ← → choose   │
╰────────────────────────────────────────────────╯
```

- **Enter** or **d** detaches — the safe choice, and the default.
- **q** again quits the app for good.
- **Esc** goes back to the app.

**Ctrl+Space then d** detaches straight away, without asking. Apps you run the
normal way (`python app.py`) quit with **q** as usual — there is nothing to detach from.

## The commands

| Command | Does |
|---|---|
| `stagetui start app.py` | Start an app that keeps running when you detach or close the terminal, and open it here |
| `stagetui start app.py --detached` | Start it in the background without opening it (`-d` for short) |
| `stagetui start app.py --name review` | Choose the session name (default: from the app's class name, `NightShift` → `night-shift`) |
| `stagetui start app.py -- --fast` | Pass arguments to your app (after `--`) |
| `stagetui sessions` | List the running apps (`--json` for programs) |
| `stagetui attach [name]` | Show a running app in this terminal (the name can be left out when only one app runs) |
| `stagetui stop [name]` | Quit a running app (no questions asked) |

While attached, **Ctrl+Space** then **d** detaches again.

## Several windows at once

Run `stagetui attach night-shift` in a second terminal window while the first is
still attached: **both windows show the same app**, live. Type in either one.

Each window is drawn with **its own terminal's abilities**: a Kitty window shows
real pictures, a simpler terminal shows the same pictures with block characters.

The app takes the size of the window you used last (like tmux). A bigger window
shows empty space around it; a smaller one shows the top-left part until you
type in it.

## Your apps work this way already

You don't need to change your app. `stagetui start` runs your file
(`python app.py`) in the background, so your `MyApp().run()` works as usual.

Things to know:

- **`print()` goes to a log file** while the app runs in the background (the
  screen belongs to stagetui). If the app crashes, `stagetui start` shows the
  last lines of that log so you can see why.
- Running an app the normal way (`python app.py`) keeps it in that terminal.
  It can't detach (**Ctrl+Space d** tells you so), but other terminals can still
  `stagetui attach` to it and watch along.
- Closing the terminal of a `python app.py` run still ends the app, like any
  program. Use `stagetui start` for things that must keep going.

## In your code

`app.session` tells you the session name while the app runs, and
`stagetui.sessions()` lists every running one:

```python
import stagetui as ts

for s in ts.sessions():
    print(s["name"], s["pid"], s["app"])
```

Long-running work — an agent loop, watching files — belongs in
`self.background(...)`, so the app keeps responding while it runs:

```python
import asyncio

from stagetui import StageApp, StageLog


class Watcher(StageApp):
    def compose(self):
        self.log = StageLog(grow=1)
        return [self.log]

    def on_mount(self):
        self.background(self.watch())

    async def watch(self):
        for n in range(3):
            self.log.append(f"check number {n + 1}: all good")
            await asyncio.sleep(1)


Watcher().run()
```

The work is cancelled when the app quits.

## Is it safe?

- The connection is a **private local socket** (a file only you can open, in
  your user's runtime folder; a named pipe on Windows). Nothing goes over the network.
- Every session has a **secret token**. A window must know it before it sees
  anything or can press a key. Only your user account can read the token.
- `stagetui sessions` cleans up entries left behind by apps that crashed.

## Good to know

| Setting | Effect |
|---|---|
| `STAGETUI_NO_SESSION=1` | Don't let other terminals attach to apps started with `python app.py` |
| `STAGETUI_SESSION=name` | The session name to use (same as `--name`) |

Popping a **single pane** out into its own terminal window — and snapping it back
when you close that window — is *coming soon* (Milestone 5b).
