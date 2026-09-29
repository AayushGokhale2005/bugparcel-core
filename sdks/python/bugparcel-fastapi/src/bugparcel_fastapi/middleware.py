"""Local-first FastAPI failure capture for BugParcel."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import uuid
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Sequence
from urllib.parse import parse_qsl, urlencode

from fastapi import Request
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.types import ASGIApp


@dataclass(frozen=True)
class BugParcelSettings:
    """The local execution contract used when a FastAPI request fails."""

    project_root: Path
    reproduction_command: Sequence[str]
    parcel_store: Path
    cli_command: Sequence[str]
    capture_timeout_seconds: float = 30
    redact_headers: frozenset[str] = field(
        default_factory=lambda: frozenset({"authorization", "cookie", "set-cookie", "x-api-key"})
    )
    redact_query_keys: frozenset[str] = field(
        default_factory=lambda: frozenset({"token", "secret", "password", "api_key"})
    )

    def __post_init__(self) -> None:
        if not self.reproduction_command:
            raise ValueError("reproduction_command must not be empty")
        if not self.cli_command:
            raise ValueError("cli_command must not be empty")


class BugParcelFastAPIMiddleware(BaseHTTPMiddleware):
    """Capture unhandled FastAPI errors while preserving the original exception."""

    def __init__(self, app: ASGIApp, *, settings: BugParcelSettings) -> None:
        super().__init__(app)
        self.settings = settings

    async def dispatch(self, request: Request, call_next):  # type: ignore[no-untyped-def]
        try:
            return await call_next(request)
        except Exception as error:
            if os.environ.get("BUGPARCEL_REPLAY") != "1":
                await asyncio.to_thread(self._capture, request, error)
            raise

    def _capture(self, request: Request, error: Exception) -> None:
        event_path = self._write_event(request, error)
        name = f"fastapi-{request.method.lower()}-{uuid.uuid4().hex[:8]}"
        command = [*self.settings.cli_command, "capture", "--name", name, "--", *self.settings.reproduction_command]
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
        receipt = {
            "capture_command": command,
            "capture_exit_code": completed.returncode,
            "capture_stdout": completed.stdout.strip(),
            "capture_stderr": completed.stderr.strip(),
        }
        with event_path.open("a", encoding="utf-8") as handle:
            handle.write("\n")
            json.dump(receipt, handle, sort_keys=True)
            handle.write("\n")

    def _write_event(self, request: Request, error: Exception) -> Path:
        events = self.settings.parcel_store / "fastapi-events"
        events.mkdir(parents=True, exist_ok=True)
        event_path = events / f"{datetime.now(UTC).strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex}.json"
        headers = {
            key: "[REDACTED]" if key.lower() in self.settings.redact_headers else value
            for key, value in request.headers.items()
        }
        query = urlencode(
            [
                (key, "[REDACTED]" if key.lower() in self.settings.redact_query_keys else value)
                for key, value in parse_qsl(request.url.query, keep_blank_values=True)
            ]
        )
        event = {
            "captured_at": datetime.now(UTC).isoformat(),
            "request": {"method": request.method, "path": request.url.path, "query": query, "headers": headers},
            "error": {"type": type(error).__name__, "message": str(error)},
            "reproduction_command": list(self.settings.reproduction_command),
        }
        event_path.write_text(json.dumps(event, indent=2, sort_keys=True), encoding="utf-8")
        return event_path
