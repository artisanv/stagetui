"""Every ```python example in README.md and docs/ must actually run.

Examples run headless in a temporary folder that contains the sample files the
docs mention (logo.png, chart.svg). `StageApp.run()` becomes `run_headless()`.
Put `<!-- not-tested -->` on the line before a block to skip it.
"""

import re
from pathlib import Path

import pytest
import stagetui as ts
from stagetui import Canvas, StageApp

ROOT = Path(__file__).parents[2]
DOCS = [ROOT / "README.md", *sorted((ROOT / "docs").glob("*.md"))]
BLOCK = re.compile(r"(<!-- not-tested -->\n)?```python\n(.*?)```", re.DOTALL)
SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" rx="8" fill="#7aa2f7"/></svg>'


def examples():
    for doc in DOCS:
        text = doc.read_text(encoding="utf-8")
        for match in BLOCK.finditer(text):
            if match.group(1):
                continue
            line = text[: match.start()].count("\n") + 1
            yield pytest.param(match.group(2), id=f"{doc.relative_to(ROOT)}:{line}")


@pytest.fixture
def sandbox(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    Canvas(64, 64, "violet").save("logo.png")
    (tmp_path / "chart.svg").write_text(SVG)
    monkeypatch.setattr(StageApp, "run", lambda self: self.run_headless())
    # Saved workspaces and sessions go to the sandbox, never the real home folder.
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    monkeypatch.setenv("APPDATA", str(tmp_path / "config"))
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(tmp_path / "run"))
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "run"))
    return tmp_path


def test_docs_have_examples():
    assert len(list(examples())) >= 20


@pytest.mark.parametrize("code", list(examples()))
def test_example_runs(code, sandbox, capsys):
    # Running the documented examples is the point of this test.
    exec(compile(code, "<doc example>", "exec"), {"__name__": "__main__", "ts": ts})  # noqa: S102
