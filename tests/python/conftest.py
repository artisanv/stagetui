import pytest


@pytest.fixture(autouse=True)
def _headless(monkeypatch: pytest.MonkeyPatch) -> None:
    """Never probe the developer's real terminal from tests."""
    monkeypatch.setenv("STAGETUI_HEADLESS", "1")
    monkeypatch.delenv("STAGETUI_SIZE", raising=False)
