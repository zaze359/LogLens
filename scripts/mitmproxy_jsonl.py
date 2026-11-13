"""mitmproxy add-on: write one JSON object per line for LogLens to import.

The desktop client imports HAR 1.2 and this JSONL format. This add-on is the bridge
for the MITM route (native OkHttp requests that WebView CDP cannot see):

    LOGLENS_JSONL=~/capture.jsonl mitmdump -s scripts/mitmproxy_jsonl.py -p 8080

Then set the device proxy to this machine (see the network panel's advanced section),
reproduce the problem, and import `capture.jsonl` with 导入 HAR / JSONL.

Privacy: credentials are redacted again inside LogLens on import, but this file is raw.
Keep it local and delete it when the investigation is over.
"""

import json
import os
from datetime import datetime, timezone
from pathlib import Path

from mitmproxy import http

OUTPUT = Path(os.environ.get("LOGLENS_JSONL", "capture.jsonl")).expanduser()
MAX_BODY = 64 * 1024


def _headers(source):
    return [{"name": k, "value": v} for k, v in (source or {}).items()]


def _text(raw, content_type=""):
    if not raw:
        return ""
    if "json" not in content_type and "text" not in content_type and "xml" not in content_type:
        return ""
    try:
        value = raw[:MAX_BODY].decode("utf-8", errors="replace")
    except Exception:
        return ""
    return value


def _write(row):
    with OUTPUT.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def _base(flow: http.HTTPFlow):
    request = flow.request
    started = datetime.fromtimestamp(request.timestamp_start, tz=timezone.utc)
    response = flow.response
    duration = 0
    if response and response.timestamp_end:
        duration = int((response.timestamp_end - request.timestamp_start) * 1000)
    return {
        "startedAt": started.isoformat(),
        "method": request.method,
        "url": request.pretty_url,
        "host": request.pretty_host,
        "path": request.path,
        "scheme": request.scheme,
        "durationMs": max(0, duration),
        "requestHeaders": _headers(request.headers),
        "responseHeaders": _headers(response.headers if response else None),
        "requestBody": _text(request.raw_content, request.headers.get("content-type", "")),
        "requestSize": len(request.raw_content or b""),
        "responseBody": "",
        "responseSize": 0,
        "status": response.status_code if response else 0,
        "statusText": response.reason if response else "",
        "mimeType": response.headers.get("content-type", "") if response else "",
        "error": "",
        "source": "jsonl",
    }


def response(flow: http.HTTPFlow):
    if not flow.response:
        return
    row = _base(flow)
    row["responseBody"] = _text(flow.response.raw_content, row["mimeType"])
    row["responseSize"] = len(flow.response.raw_content or b"")
    _write(row)


def error(flow: http.HTTPFlow):
    # TLS failures, DNS errors and connection resets have no response object, but they
    # are often the most interesting rows in a mobile bug report.
    row = _base(flow)
    row["error"] = str(flow.error) if flow.error else "请求失败"
    _write(row)


