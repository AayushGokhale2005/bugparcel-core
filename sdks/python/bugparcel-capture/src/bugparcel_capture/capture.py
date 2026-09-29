"""Portable failure capture for any Python backend runtime.

The module deliberately knows nothing about a framework. Its middleware wrappers
only translate ASGI or WSGI requests into the same sanitized event that a manual
``BugParcelCapture.capture_exception`` call can emit.
"""

from __future__ import annotations

import asyncio
import io
import json
import os
import subprocess
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence
from urllib.parse import parse_qsl, urlencode


@dataclass(frozen=True)
class BugParcelSettings:
    """The local replay contract shared by every framework adapter."""

    project_root: Path
    reproduction_command: Sequence[str]
    parcel_store: Path
    cli_command: Sequence[str]
    fixture_files: Sequence[Path] = ()
    capture_timeout_seconds: float = 30
    redact_headers: frozenset[str] = field(
        default_factory=lambda: frozenset({"authorization", "cookie", "set-cookie", "x-api-key"})
    )
    redact_query_keys: frozenset[str] = field(
        default_factory=lambda: frozenset({"token", "secret", "password", "api_key"})
    )
    redact_body_keys: frozenset[str] = field(
        default_factory=lambda: frozenset({"token", "secret", "password", "api_key", "authorization"})
    )

    def __post_init__(self) -> None:
        if not self.reproduction_command:
            raise ValueError("reproduction_command must not be empty")
        if not self.cli_command:
            raise ValueError("cli_command must not be empty")


@dataclass(frozen=True)
class HttpRequest:
    """The smallest portable HTTP request representation needed for replay."""

    method: str
    path: str
    query: str = ""
    headers: Mapping[str, str] = field(default_factory=dict)
    body: bytes = b""


class BugParcelCapture:
    """Capture an exception from any Python server or worker entry point."""

    def __init__(self, settings: BugParcelSettings) -> None:
        self.settings = settings

    def capture_exception(
        self,
        error: Exception,
        *,
        source: str,
        request: HttpRequest | None = None,
    ) -> Path:
        """Write a sanitized event and ask the local CLI to create a parcel.

        Capture errors intentionally propagate here. Middleware wrappers catch
        them so they can never hide the backend exception that triggered capture.
        """

        event_path = self._write_event(source=source, request=request, error=error)
        method = request.method.lower() if request else "error"
        command = [
            *self.settings.cli_command,
            "capture",
            "--name",
            f"{source}-{method}-{uuid.uuid4().hex[:8]}",
            "--expect-output",
            type(error).__name__,
            "--contract-file",
            str(event_path),
            "--state-file",
            str(event_path),
            "--state-json-pointer",
            "/request/body",
            *[item for path in self.settings.fixture_files for item in ("--fixture-file", str(path))],
            "--env",
            f"BUGPARCEL_CAPTURED_FROM={source}",
            "--",
            *self.settings.reproduction_command,
        ]
        environment = {**os.environ, "BUGPARCEL_HOME": str(self.settings.parcel_store)}
        completed = subprocess.run(
            command,
            cwd=self.settings.project_root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=self.settings.capture_timeout_seconds,
            check=False,
        )
        event = json.loads(event_path.read_text(encoding="utf-8"))
        receipt: dict[str, Any] = {
            "capture_command": command,
            "capture_exit_code": completed.returncode,
            "capture_stdout": completed.stdout.strip(),
            "capture_stderr": completed.stderr.strip(),
        }
        for token in completed.stdout.split():
            if token.startswith("bp_"):
                receipt["parcel_id"] = token
                break
        event["capture"] = receipt
        event_path.write_text(json.dumps(event, indent=2, sort_keys=True), encoding="utf-8")
        return event_path

    def _write_event(self, *, source: str, request: HttpRequest | None, error: Exception) -> Path:
        events = self.settings.parcel_store / "events"
        events.mkdir(parents=True, exist_ok=True)
        event_path = events / f"{datetime.now(UTC).strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex}.json"
        event = {
            "captured_at": datetime.now(UTC).isoformat(),
            "source": source,
            "request": self._request_payload(request),
            "error": {"type": type(error).__name__, "message": str(error)},
            "reproduction_command": list(self.settings.reproduction_command),
        }
        event_path.write_text(json.dumps(event, indent=2, sort_keys=True), encoding="utf-8")
        return event_path

    def _request_payload(self, request: HttpRequest | None) -> dict[str, Any]:
        if request is None:
            return {"method": None, "path": None, "query": "", "headers": {}, "body": None}
        headers = {
            key: "[REDACTED]" if key.lower() in self.settings.redact_headers else value
            for key, value in request.headers.items()
        }
        query = urlencode(
            [
                (key, "[REDACTED]" if key.lower() in self.settings.redact_query_keys else value)
                for key, value in parse_qsl(request.query, keep_blank_values=True)
            ]
        )
        return {
            "method": request.method,
            "path": request.path,
            "query": query,
            "headers": headers,
            "body": self._sanitize_body(request.body),
        }

    def _sanitize_body(self, body: bytes) -> Any:
        if not body:
            return None
        try:
            return self._redact_json(json.loads(body))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return {"encoding": "non-json", "byte_length": len(body)}

    def _redact_json(self, value: Any) -> Any:
        if isinstance(value, dict):
            return {
                key: "[REDACTED]" if key.lower() in self.settings.redact_body_keys else self._redact_json(item)
                for key, item in value.items()
            }
        if isinstance(value, list):
            return [self._redact_json(item) for item in value]
        return value


class BugParcelASGIMiddleware:
    """ASGI wrapper for Starlette, FastAPI, Django ASGI, Quart, and custom apps."""

    def __init__(self, app: Callable[..., Any], *, settings: BugParcelSettings) -> None:
        self.app = app
        self.capture = BugParcelCapture(settings)

    async def __call__(self, scope: Mapping[str, Any], receive: Callable[..., Any], send: Callable[..., Any]) -> None:
        if scope.get("type") != "http":
            await self.app(scope, receive, send)
            return

        chunks: list[bytes] = []

        async def recording_receive() -> Any:
            message = await receive()
            if message.get("type") == "http.request":
                chunks.append(message.get("body", b""))
            return message

        try:
            await self.app(scope, recording_receive, send)
        except Exception as error:
            if os.environ.get("BUGPARCEL_REPLAY") != "1":
                request = HttpRequest(
                    method=str(scope.get("method", "GET")),
                    path=str(scope.get("path", "/")),
                    query=bytes(scope.get("query_string", b"")).decode("latin-1"),
                    headers={
                        bytes(key).decode("latin-1"): bytes(value).decode("latin-1")
                        for key, value in scope.get("headers", [])
                    },
                    body=b"".join(chunks),
                )
                await asyncio.to_thread(self._capture_without_masking_error, error, "asgi", request)
            raise

    def _capture_without_masking_error(self, error: Exception, source: str, request: HttpRequest) -> None:
        try:
            self.capture.capture_exception(error, source=source, request=request)
        except Exception:
            pass


class BugParcelWSGIMiddleware:
    """WSGI wrapper for Flask, Django WSGI, Bottle, and custom apps."""

    def __init__(self, app: Callable[..., Iterable[bytes]], *, settings: BugParcelSettings) -> None:
        self.app = app
        self.capture = BugParcelCapture(settings)

    def __call__(self, environ: dict[str, Any], start_response: Callable[..., Any]) -> Iterable[bytes]:
        body = environ.get("wsgi.input", io.BytesIO()).read()
        environ["wsgi.input"] = io.BytesIO(body)
        request = HttpRequest(
            method=str(environ.get("REQUEST_METHOD", "GET")),
            path=str(environ.get("PATH_INFO", "/")),
            query=str(environ.get("QUERY_STRING", "")),
            headers={
                key[5:].replace("_", "-").lower(): str(value)
                for key, value in environ.items()
                if key.startswith("HTTP_")
            },
            body=body,
        )
        try:
            response = self.app(environ, start_response)
        except Exception as error:
            self._capture_without_masking_error(error, request)
            raise

        def iterate() -> Iterable[bytes]:
            try:
                yield from response
            except Exception as error:
                self._capture_without_masking_error(error, request)
                raise
            finally:
                close = getattr(response, "close", None)
                if close:
                    close()

        return iterate()

    def _capture_without_masking_error(self, error: Exception, request: HttpRequest) -> None:
        if os.environ.get("BUGPARCEL_REPLAY") == "1":
            return
        try:
            self.capture.capture_exception(error, source="wsgi", request=request)
        except Exception:
            pass
