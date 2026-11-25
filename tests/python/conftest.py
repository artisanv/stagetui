import pytest


@pytest.fixture(autouse=True)
def _headless(monkeypatch: pytest.MonkeyPatch) -> None:
    """Never probe the developer's real terminal from tests."""
    monkeypatch.setenv("TERMSTAGE_HEADLESS", "1")
    monkeypatch.delenv("TERMSTAGE_SIZE", raising=False)
