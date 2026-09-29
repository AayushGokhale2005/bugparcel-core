from pathlib import Path

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from bugparcel_fastapi import BugParcelFastAPIMiddleware, BugParcelSettings


def test_adapter_writes_redacted_event_and_preserves_exception(tmp_path: Path) -> None:
    app = FastAPI()
    settings = BugParcelSettings(
        project_root=tmp_path,
        reproduction_command=["python", "-m", "pytest", "-q"],
        parcel_store=tmp_path / "store",
        cli_command=["echo"],
    )
    app.add_middleware(BugParcelFastAPIMiddleware, settings=settings)

    @app.get("/boom")
    def boom() -> None:
        raise RuntimeError("checkout failed")

    with pytest.raises(RuntimeError, match="checkout failed"):
        TestClient(app).get("/boom?token=keep-private", headers={"Authorization": "Bearer secret"})

    event_files = list((tmp_path / "store" / "fastapi-events").glob("*.json"))
    assert len(event_files) == 1
    event = event_files[0].read_text(encoding="utf-8")
    assert "Bearer secret" not in event
    assert "keep-private" not in event
    assert "[REDACTED]" in event
