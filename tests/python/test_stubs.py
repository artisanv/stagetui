"""`_core.pyi` must describe exactly what the extension exports."""

import ast
from pathlib import Path

import termstage._core as core

STUB = Path(__file__).parents[2] / "python" / "termstage" / "_core.pyi"


def stub_names() -> set[str]:
    tree = ast.parse(STUB.read_text())
    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return names


def test_stub_matches_extension():
    exported = {n for n in dir(core) if not n.startswith("__") or n == "__version__"}
    exported -= {
        "__builtins__",
        "__doc__",
        "__file__",
        "__loader__",
        "__name__",
        "__package__",
        "__spec__",
        "__path__",
        "__all__",
    }
    assert stub_names() == exported
