import json
import os
import shutil
import subprocess
import sys

import stagetui


def run(*args: str) -> subprocess.CompletedProcess[str]:
    env = {**os.environ, "STAGETUI_HEADLESS": "1"}
    return subprocess.run(
        args,
        check=False,
        capture_output=True,
        encoding="utf-8",
        env=env,
        stdin=subprocess.DEVNULL,
        timeout=60,
    )


def test_console_script_inspect_json():
    exe = shutil.which("stagetui")
    assert exe, "the `stagetui` console script is not installed"
    out = run(exe, "inspect", "--json")
    assert out.returncode == 0, out.stderr
    caps = json.loads(out.stdout)
    assert caps["version"] == stagetui.__version__
    assert caps["headless"] is True


def test_python_dash_m_doctor():
    out = run(sys.executable, "-m", "stagetui", "doctor")
    assert out.returncode == 0, out.stderr
    assert "stagetui doctor" in out.stdout and "headless" in out.stdout


def test_cli_library_parity():
    lib = stagetui.inspect()
    cli = json.loads(run(sys.executable, "-m", "stagetui", "inspect", "--compact").stdout)
    for key in ("headless", "graphics", "keyboard", "truecolor", "platform", "version"):
        assert lib[key] == cli[key], key


def test_bad_usage_exit_code():
    out = run(sys.executable, "-m", "stagetui", "frobnicate")
    assert out.returncode == 2
    assert "frobnicate" in out.stderr


def test_render_text_card(tmp_path):
    png, txt, tree = tmp_path / "c.png", tmp_path / "c.txt", tmp_path / "c.json"
    out = run(
        sys.executable,
        "-m",
        "stagetui",
        "render",
        "مرحبا بكم",
        "--dir",
        "rtl",
        "--title",
        "عربي",
        "--out",
        str(png),
        "--text",
        str(txt),
        "--tree",
        str(tree),
    )
    assert out.returncode == 0, out.stderr
    assert png.read_bytes().startswith(b"\x89PNG")
    assert f"saved {png}" in out.stderr, "render must say where it saved files"
    assert "مرحبا بكم" in txt.read_text(encoding="utf-8")
    assert json.loads(tree.read_text(encoding="utf-8"))["children"][0]["dir"] == "rtl"


def test_render_app_file(tmp_path):
    png = tmp_path / "hello.png"
    root = __import__("pathlib").Path(__file__).parents[2]
    out = run(
        sys.executable,
        "-m",
        "stagetui",
        "render",
        str(root / "examples" / "01_hello.py"),
        "--size",
        "60x20",
        "--out",
        str(png),
    )
    assert out.returncode == 0, out.stderr
    assert png.read_bytes().startswith(b"\x89PNG")
    printed = run(
        sys.executable, "-m", "stagetui", "render", str(root / "examples" / "01_hello.py")
    )
    assert "Welcome to stagetui" in printed.stdout


def test_render_svg_to_png(tmp_path):
    svg = tmp_path / "s.svg"
    svg.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>'
    )
    out = run(
        sys.executable, "-m", "stagetui", "render", str(svg), "--out", str(tmp_path / "s.png")
    )
    assert out.returncode == 0, out.stderr
    assert (tmp_path / "s.png").read_bytes().startswith(b"\x89PNG")


def test_render_errors_are_friendly(tmp_path):
    missing = run(sys.executable, "-m", "stagetui", "render", str(tmp_path / "nope.py"))
    assert missing.returncode == 1 and "fix:" in missing.stderr
    no_app = tmp_path / "plain.py"
    no_app.write_text("x = 1\n")
    out = run(sys.executable, "-m", "stagetui", "render", str(no_app))
    assert out.returncode != 0 and "StageApp" in out.stderr


def test_show_text_cli():
    out = run(sys.executable, "-m", "stagetui", "show", "Hello مرحبا", "--title", "Hi")
    assert out.returncode == 0, out.stderr
    assert "Hello مرحبا" in out.stdout and "Hi" in out.stdout
