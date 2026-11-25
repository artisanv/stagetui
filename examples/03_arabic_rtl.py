"""Arabic, Persian, Hindi and Japanese in one app — shaped and ordered correctly.

Run:   python examples/03_arabic_rtl.py
Quit:  q or Esc

Try it in different terminals (Kitty, Ghostty, GNOME Terminal, Windows Terminal…).
Also try:  termstage render examples/03_arabic_rtl.py --out world.png
"""

from termstage import StageApp, StageCard


class WorldScripts(StageApp):
    theme = "velvet-dusk"

    def compose(self):
        return [
            StageCard("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", title="العربية", icon="sparkles"),
            StageCard("سلام دنیا، گل‌های زیبا", title="فارسی", accent="mint"),
            StageCard("Mixed: Hello مرحبا (123) world", title="Mixed directions"),
            StageCard("नमस्ते दुनिया · こんにちは世界", title="Hindi & Japanese", accent="amber"),
        ]


if __name__ == "__main__":
    WorldScripts().run()
