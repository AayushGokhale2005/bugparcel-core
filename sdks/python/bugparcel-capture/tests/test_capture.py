import asyncio
import io
import json
from pathlib import Path

import pytest

from bugparcel_capture import (
    BugParcelASGIMiddleware,
    BugParcelCapture,
    BugParcelSettings,
    BugParcelWSGIMiddleware,
    HttpRequest,
)


def settings(tmp_path: Path) -> BugParcelSettings:
    return BugParcelSettings(
        project_root=tmp_path,
        reproduction_command=["python", "-m", "pytest", "-q"],
        parcel_store=tmp_path / "store",
        cli_command=["echo"],
    )


def test_manual_capture_is_framework_neutral_and_redacts_data(tmp_path: Path) -> None:
    event_path = BugParcelCapture(settings(tmp_path)).capture_exception(
        RuntimeError("checkout failed"),
        source="flask",
        request=HttpRequest(
            method="POST",
            path="/checkout",
            query="token=private&mode=live",
            headers={"Authorization": "Bearer private"},
            body=b'{"token":"secret","cart":882}',
        ),
    )
    event = json.loads(event_path.read_text(encoding="utf-8"))
    serialized = json.dumps(event)
    assert event["source"] == "flask"
    assert event["error"]["type"] == "RuntimeError"
    assert "Bearer private" not in serialized
    assert "secret" not in serialized
    assert "token=private" not in serialized
    assert event["request"]["body"] == {"token": "[REDACTED]", "cart": 882}
    assert "BUGPARCEL_CAPTURED_FROM=flask" in event["capture"]["capture_command"]


def test_asgi_middleware_captures_without_framework_dependency(tmp_path: Path) -> None:
    async def app(scope, receive, send):
        await receive()
        raise ValueError("broken route")

    middleware = BugParcelASGIMiddleware(app, settings=settings(tmp_path))

    async def receive():
        return {"type": "http.request", "body": b'{"password":"private"}', "more_body": False}

    async def send(message):
        return None

    with pytest.raises(ValueError, match="broken route"):
        asyncio.run(
            middleware(
                {
                    "type": "http",
                    "method": "POST",
                    "path": "/boom",
                    "query_string": b"token=private",
                    "headers": [(b"authorization", b"Bearer private")],
                },
                receive,
                send,
            )
        )

    event = json.loads(next((tmp_path / "store" / "events").glob("*.json")).read_text())
    assert event["source"] == "asgi"
    assert event["request"]["body"] == {"password": "[REDACTED]"}


def test_wsgi_middleware_preserves_the_original_exception(tmp_path: Path) -> None:
    def app(environ, start_response):
        raise LookupError("missing order")

    middleware = BugParcelWSGIMiddleware(app, settings=settings(tmp_path))
    with pytest.raises(LookupError, match="missing order"):
        middleware(
            {
                "REQUEST_METHOD": "POST",
                "PATH_INFO": "/orders",
                "QUERY_STRING": "api_key=private",
                "HTTP_X_API_KEY": "private",
                "wsgi.input": io.BytesIO(b'{"debug":true}'),
            },
            lambda status, headers: None,
        )
    event = json.loads(next((tmp_path / "store" / "events").glob("*.json")).read_text())
    assert event["source"] == "wsgi"
    assert event["request"]["headers"]["x-api-key"] == "[REDACTED]"
