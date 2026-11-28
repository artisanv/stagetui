"""A mini coding-agent harness — chat, proposed changes, a real shell — with a
pretend agent (no AI is connected, so it runs anywhere, offline).

Run:   python examples/07_harness.py
Try:   type a question and press Enter · /run ls -la · /diff · /clear · @ to mention a file
Keys:  Tab moves between panes · in the shell, Ctrl+Space then Tab gets you out
       click a pane's title to switch what it shows · Ctrl+T theme
       Ctrl+Space then q quits from anywhere (q alone types into the prompt or shell)
"""

import asyncio
import os

from stagetui import StageApp, StageBar, StagePane, StagePrompt, StageSplit

BEFORE = """def greet(name):
    print("hello " + name)


def farewell(name):
    print("bye " + name)


def shout(text):
    print(text.upper())


def main():
    greet("world")
"""

AFTER = """def greet(name: str) -> None:
    print(f"Hello, {name}!")


def farewell(name):
    print("bye " + name)


def shout(text):
    print(text.upper())


def main() -> None:
    greet("world")
    greet("مرحبا")
"""


class Harness(StageApp):
    theme = "aurora-dark"
    accent = "violet"
    BINDINGS = [
        ("ctrl+t", "next_theme", "Theme"),
        ("ctrl+l", "clear", "Clear chat"),
    ]

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
            StagePrompt(
                placeholder="Ask the (pretend) agent…   / for commands · @ to mention a file",
                commands=[
                    ("help", "What can I do?"),
                    ("run", "Run a command in the shell (asks first)"),
                    ("diff", "Propose a change to a file"),
                    ("clear", "Clear the conversation"),
                ],
                on_submit=self.ask,
            ),
            StageBar("Pretend agent · no AI connected"),
        ]

    @property
    def chat(self):
        return self.query("#chat").content

    def on_mount(self):
        msg = self.chat.add("assistant")
        msg.update(
            "# Welcome\n\n"
            "This is a **pretend** agent, so you can try the harness offline.\n\n"
            "- Ask anything and watch the answer *stream* in.\n"
            "- `/run ls -la` asks permission, then runs it in the shell on the right.\n"
            "- `/diff` proposes a change — press **a** to accept or **r** to reject each part.\n"
            "- Click any pane's title to show something else there (Blender-style)."
        )

    async def ask(self, text):
        text = text.strip()
        if not text:
            return
        if text.startswith("/"):
            command, _, rest = text[1:].partition(" ")
            await self.command(command, rest.strip())
            return
        self.chat.add("user", text)
        # A tool call first, like a real agent looking around.
        tool = self.chat.add_tool("list_files", {"path": "."})
        await asyncio.sleep(0.3)
        files = sorted(f for f in os.listdir(".") if not f.startswith("."))
        tool.finish("\n".join(files[:20]))
        reply = self.chat.add("assistant")
        answer = (
            f"You asked: *{text}*\n\n"
            f"I looked around and found **{len(files)}** items here. "
            "A real agent would now read code and propose changes. For example:\n\n"
            "```python\n"
            "def answer():\n"
            "    return 42\n"
            "```\n\n"
            "Try `/diff` to see a proposed change."
        )
        for word in answer.split(" "):
            reply.append(word + " ")
            await asyncio.sleep(0.03)

    async def command(self, name, rest):
        if name == "clear":
            self.action_clear()
        elif name == "help":
            self.chat.add(
                "assistant",
                "Commands: `/run <command>`, `/diff`, `/clear`. Type `@` to mention a file.",
            )
        elif name == "run":
            cmd = rest or "ls -la"
            choice = await self.ask_permission(
                "Run this command in the shell?", tool="bash", args={"cmd": cmd}
            )
            if choice == "deny":
                self.chat.add("system", f"Denied: `{cmd}`")
                return
            self.query("#shell").content.send(cmd + "\n")
            self.chat.add("system", f"Ran `{cmd}` in the shell →")
        elif name == "diff":
            self.query("#changes").set_type("diff", old=BEFORE, new=AFTER, path="hello.py")
            self.chat.add(
                "assistant",
                "I propose two changes to `hello.py` (on the right). "
                "Focus the diff and press **a** to accept or **r** to reject each part.",
            )
        else:
            self.notify(f"Unknown command /{name}", icon="warning")

    def action_clear(self):
        self.chat.clear()
        self.notify("Chat cleared", icon="check")


if __name__ == "__main__":
    Harness().run()
