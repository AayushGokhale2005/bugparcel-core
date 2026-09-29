import json
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

    @app.post("/boom")
    def boom() -> None:
        raise RuntimeError("checkout failed")

    with pytest.raises(RuntimeError, match="checkout failed"):
        TestClient(app).post(
            "/boom?token=keep-private",
            headers={"Authorization": "Bearer secret"},
            json={"token": "body-secret", "debug": "keep"},
        )

    event_files = list((tmp_path / "store" / "fastapi-events").glob("*.json"))
    assert len(event_files) == 1
    event = json.loads(event_files[0].read_text(encoding="utf-8"))
    serialized = json.dumps(event)
    assert "Bearer secret" not in serialized
    assert "keep-private" not in serialized
    assert "body-secret" not in serialized
    assert "[REDACTED]" in serialized
    assert event["error"]["type"] == "RuntimeError"
