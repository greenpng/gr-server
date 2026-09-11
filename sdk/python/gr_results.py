#!/usr/bin/env python3
"""greenpng results SDK (Python) — backend only.

Thin client: get_result / wait_for_result / query. No probe relay; the SDK
only reads analysis results for sessions that were probed by browser/edge.

Usage:
    from gr_results import GrResultClient, cookie_fields

    client = GrResultClient(base_url="https://probe.example.com",
                             api_key=os.environ["GR_SITE_RESULT_KEY"])
    result = client.wait_for_result("sess_...", projection="sdk")
    print(cookie_fields(result))   # {"user_id": "u9", ...} or None

Auth: X-Gr-Sdk-Key with the site-scoped backend key.
Diagnostic projection requires an ops/admin token and is refused by the server for site keys.
"""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, Optional


class GrAnalysisPending(Exception):
    """Raised by wait_for_result when the session is still pending at timeout."""

    def __init__(self, last_body: Any):
        self.last_body = last_body
        super().__init__("analysis_pending")


class GrApiError(Exception):
    def __init__(self, message: str, status: Optional[int] = None, body: Any = None):
        self.status = status
        self.body = body
        super().__init__(message)

    @property
    def code(self) -> str:
        if isinstance(self.body, dict):
            err = self.body.get("error") or {}
            if isinstance(err, dict):
                return str(err.get("code") or err.get("message") or "unknown")
            return str(err)
        return "unknown"


class GrResultClient:
    """Read-only result client bound to one probe server + one site key."""

    def __init__(
        self,
        base_url: str,
        api_key: str,
        timeout_ms: int = 15000,
        expected_schema: str = "product_public_v1",
    ):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.timeout_s = timeout_ms / 1000.0
        self.expected_schema = expected_schema

    def _headers(self) -> Dict[str, str]:
        return {
            "Accept": "application/json",
            "X-Gr-Sdk-Key": self.api_key,
            "X-Request-Id": f"req_{int(time.time() * 1000)}",
        }

    def _fetch(self, url: str) -> Any:
        req = urllib.request.Request(url, headers=self._headers())
        try:
            with urllib.request.urlopen(req, timeout=self.timeout_s) as resp:
                return json.loads(resp.read().decode("utf-8") or "{}")
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", "replace") or "{}"
            try:
                body = json.loads(raw)
            except ValueError:
                body = {"error": raw}
            status = e.code
            if isinstance(body.get("error"), str) and not body["error"].startswith("http_"):
                raise GrApiError(
                    str(body["error"]), status=status, body=body
                ) from None
            raise GrApiError(f"http_{status}", status=status, body=body) from None
        except urllib.error.URLError as e:
            raise GrApiError(str(e.reason)) from None

    def _result_url(
        self,
        session_id: str,
        projection: str,
        strategy_id: Optional[str] = None,
        response_profile: Optional[str] = None,
        profile_cap: Optional[str] = None,
        lang: Optional[str] = None,
    ) -> str:
        params = {"projection": projection}
        for key, value in (
            ("strategy_id", strategy_id),
            ("response_profile", response_profile),
            ("profile_cap", profile_cap),
            ("lang", lang),
        ):
            if value:
                params[key] = value
        return (
            f"{self.base_url}/v1/session/{urllib.parse.quote(session_id, safe='')}"
            f"/result?{urllib.parse.urlencode(params)}"
        )

    def get_result(
        self,
        session_id: str,
        projection: str = "sdk",
        strategy_id: Optional[str] = None,
        response_profile: Optional[str] = None,
        profile_cap: Optional[str] = None,
        lang: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Fetch one result snapshot (projection: public|sdk|diagnostic)."""
        body = self._fetch(self._result_url(
            session_id, projection, strategy_id, response_profile, profile_cap, lang
        ))
        if isinstance(body, dict) and self.expected_schema and \
                body.get("schema_version") and \
                body["schema_version"] != self.expected_schema:
            pass  # tolerate older nodes without schema_version
        return body

    def wait_for_result(
        self,
        session_id: str,
        timeout_ms: int = 8000,
        interval_ms: int = 500,
        projection: str = "sdk",
        strategy_id: Optional[str] = None,
        response_profile: Optional[str] = None,
        profile_cap: Optional[str] = None,
        lang: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Poll until the analysis is no longer pending or timeout_ms elapses."""
        deadline = time.monotonic() + timeout_ms / 1000.0
        last: Any = None
        while True:
            last = self.get_result(
                session_id,
                projection=projection,
                strategy_id=strategy_id,
                response_profile=response_profile,
                profile_cap=profile_cap,
                lang=lang,
            )
            if isinstance(last, dict) and last.get("ok"):
                has_body = bool(last.get("product_public") or last.get("sdk_projection"))
                pending = False
                if isinstance(last.get("product_public"), dict):
                    meta = last["product_public"].get("meta") or {}
                    pending = bool(meta.get("analysis_pending"))
                if has_body and not pending:
                    return last
            if time.monotonic() >= deadline:
                raise GrAnalysisPending(last)
            time.sleep(interval_ms / 1000.0)

    def query(
        self,
        session_id: str,
        projection: str = "sdk",
        wait: bool = False,
        timeout_ms: int = 8000,
        interval_ms: int = 500,
        strategy_id: Optional[str] = None,
        response_profile: Optional[str] = None,
        profile_cap: Optional[str] = None,
        lang: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Query a session result; `wait=True` polls until ready."""
        if wait:
            return self.wait_for_result(
                session_id,
                timeout_ms=timeout_ms,
                interval_ms=interval_ms,
                projection=projection,
                strategy_id=strategy_id,
                response_profile=response_profile,
                profile_cap=profile_cap,
                lang=lang,
            )
        return self.get_result(
            session_id,
            projection=projection,
            strategy_id=strategy_id,
            response_profile=response_profile,
            profile_cap=profile_cap,
            lang=lang,
        )


def cookie_fields(result: Any) -> Optional[Dict[str, str]]:
    """Extract site-allowlisted cookies captured server-side (business ids).

    Reads sdk_projection.cookie_fields, else product_public.cookie_fields,
    else top-level cookie_fields. Returns None when nothing was captured.
    """
    if not isinstance(result, dict):
        return None
    for path in ("sdk_projection", "product_public"):
        node = result.get(path)
        if isinstance(node, dict) and isinstance(node.get("cookie_fields"), dict):
            cf = node["cookie_fields"]
            return cf if cf else None
    if isinstance(result.get("cookie_fields"), dict):
        cf = result["cookie_fields"]
        return cf if cf else None
    return None
