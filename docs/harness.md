# Build an AI agent harness

A **harness** is the app you use to talk to an AI coding agent: a chat, a box to
type in, the changes the agent proposes, the commands it runs. stagetui has a
widget for each part. You connect your own AI (any library, any provider);
stagetui does the screen.

`examples/07_harness.py` is a complete harness with a pretend agent — run it to
see everything on this page working together.

## The conversation: `StageChat` + `StagePrompt`

```python
import asyncio

from stagetui import StageApp, StageChat, StagePrompt


class MiniAgent(StageApp):
    def compose(self):
        return [
            StageChat(id="chat", height=20),
            StagePrompt(placeholder="Ask anything…", on_submit=self.ask),
        ]

    async def ask(self, text):
        chat = self.query("#chat")
        chat.add("user", text)
        reply = chat.add("assistant")  # an empty message that fills as the AI writes
        for word in ["Here", " is", " **my**", " answer."]:
            reply.append(word)
            await asyncio.sleep(0.05)  # pretend the AI is thinking


MiniAgent().run()
```

- `chat.add(role, text)` adds a message. Roles: `"user"`, `"assistant"`, `"system"`.
- It returns a **message** you can keep writing to: `msg.append(chunk)` (streaming)
  or `msg.update(text)` (replace).
- Assistant messages are **Markdown**: headings, **bold**, lists, tables and code
  blocks with syntax colors.
- The chat follows new text automatically. Scroll up to read; **End** follows again.
- Focus the chat and use **↑ / ↓** to select a message, **y** to copy it.

### With a real AI

Stream the AI's answer straight into `append`. With most AI libraries it looks like this:

<!-- not-tested -->
```python
async def ask(self, text):
    chat = self.query("#chat")
    chat.add("user", text)
    reply = chat.add("assistant")
    async for chunk in my_ai.stream(text):  # your AI library here
        reply.append(chunk)
```

## The prompt box: `StagePrompt`

```python
from stagetui import StagePrompt

StagePrompt(
    placeholder="Ask anything…",
    commands=[("help", "Show help"), ("clear", "Clear the chat")],  # typed as /help
    mentions=["src/app.py", "README.md"],  # offered after @ (default: files in this folder)
)
```

| Key | What it does |
|---|---|
| **Enter** | Send (`on_submit(text)`), then the box empties |
| **Shift+Enter**, **Alt+Enter**, **Ctrl+J** | New line |
| **↑ / ↓** (on the first / last line) | Previous / next thing you sent |
| **/** at the start | Commands list — **Tab** or **Enter** picks one |
| **@** | File list — keep typing to narrow it down |
| **Ctrl+U** / **Ctrl+W** | Clear everything / delete a word |

## Tool calls: `StageToolCall`

When the agent uses a tool, show it. It spins while running, then shows ✓ or ✗:

```python
from stagetui import StageApp, StageChat


class Tools(StageApp):
    def compose(self):
        return [StageChat(id="chat", height=10)]

    def on_mount(self):
        tool = self.query("#chat").add_tool("read_file", {"path": "app.py"})
        tool.finish("42 lines read")  # or tool.finish("not found", ok=False)


Tools().run()
```

Click a tool call (or select it and press Enter) to see its arguments and output.
`StageToolCall("bash", args={...})` is the same card on its own, outside a chat.

## Asking first: `ask_permission`

Before the agent does something risky, ask. A pop-up appears; the user answers
with **y** (allow), **a** (always allow) or **n** (deny):

```python
from stagetui import StageApp, StagePrompt


class Careful(StageApp):
    def compose(self):
        return [StagePrompt(placeholder="Type a command", on_submit=self.run_it)]

    async def run_it(self, command):
        answer = await self.ask_permission(
            "Run this command?", tool="bash", args={"cmd": command}
        )
        if answer == "deny":
            self.notify("Not running it", icon="x")
        else:  # "allow" or "always"
            self.notify(f"Running {command}", icon="check")


Careful().run()
```

## Proposed changes: `StageDiff`

Show what the agent wants to change. Each **hunk** (a group of changed lines) can
be accepted or rejected:

```python
from stagetui import StageApp, StageDiff

OLD = "def greet(name):\n    print('hello ' + name)\n"
NEW = "def greet(name: str) -> None:\n    print(f'Hello, {name}!')\n"


class Review(StageApp):
    def compose(self):
        return [StageDiff((OLD, NEW), path="hello.py", id="diff", on_decide=self.decided)]

    def decided(self, choice):
        diff = self.query("#diff")
        print(choice, diff.result)  # result = the text with accepted hunks applied


Review().run()
```

Keys while the diff has focus: **a** accept, **r** reject, **A** accept all,
**j / k** next / previous hunk. In code: `diff.accept_all()`, `diff.reject_all()`,
`diff.decide(0, "accept")`, and `diff.result`.

## Code and logs

```python
from stagetui import StageApp, StageCode, StageLog


class Viewer(StageApp):
    def compose(self):
        return [
            StageCode("def add(a, b):\n    return a + b\n", language="python"),
            StageLog(id="log", height=6),
        ]

    def on_mount(self):
        log = self.query("#log")
        log.append("INFO started")
        log.append(["WARN disk 91% full", "ERROR could not connect"])


Viewer().run()
```

- `StageCode` colors 150+ languages; `StageCode.from_file("app.py")` loads a file.
- `StageLog` keeps the last 5000 lines (`max_lines=`), colors ERROR / WARN / DEBUG / OK,
  follows new lines, and `log.filter("error")` shows only matching lines.

## Markdown on its own: `StageMarkdown`

`StageMarkdown(text)` shows a Markdown document; `md.append(chunk)` streams more in.

## A real shell: `StageTerminal`

`StageTerminal()` runs your shell inside the app; `StageTerminal("htop")` runs a
program. While it has focus it gets **every key** — Ctrl+C, Esc and Tab work
inside it. To reach the app, press **Ctrl+Space** first, then **Tab** to move on
(or **q** to quit). See [Panes](panes.md).

<!-- not-tested -->
```python
from stagetui import StageTerminal

shell = StageTerminal(id="shell")
shell.send("pytest -q\n")  # type into it from code
print(shell.screen)  # what it shows right now, as text
```

The program stops when your app exits — even if your app crashes.

## Putting it together

The usual harness layout is a split: the chat on one side, changes and a shell
on the other, the prompt at the bottom. With [panes](panes.md) every part can be
switched to something else while the app runs:

<!-- not-tested -->
```python
from stagetui import StageApp, StageBar, StagePane, StagePrompt, StageSplit


class Harness(StageApp):
    def compose(self):
        return [
            StageSplit(
                [
                    StagePane("chat", id="chat"),
                    StageSplit(
                        [StagePane("diff", id="changes"), StagePane("terminal", id="shell")],
                        direction="vertical",
                    ),
                ],
                ratio=[3, 2],
            ),
            StagePrompt(placeholder="Ask anything…", on_submit=self.ask),
            StageBar("Ready"),
        ]

    async def ask(self, text):
        chat = self.query("#chat").content  # the StageChat inside the pane
        chat.add("user", text)
```
