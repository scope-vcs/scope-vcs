"""The caller supplies attempt identity, never AWS execution instructions."""

import hashlib
import json
import re
import urllib.error
import urllib.request


class Denied(Exception):
    pass


class Pending(Exception):
    pass


def request(value):
    if not isinstance(value, dict):
        raise Denied("dispatch request must be an object")
    action = value.get("action")
    fields = {"action", "attempt_id"}
    if action == "start":
        fields.add("bootstrap_token")
    elif action != "stop":
        raise Denied("unsupported dispatch action")
    if set(value) != fields:
        raise Denied("dispatch request fields are invalid")
    attempt = value.get("attempt_id")
    if not isinstance(attempt, str) or not re.fullmatch(r"attempt_[0-9a-f]{32}", attempt):
        raise Denied("attempt identity is invalid")
    if action == "start":
        token = value.get("bootstrap_token")
        if not isinstance(token, str) or not re.fullmatch(r"scope_bootstrap_[0-9a-f]{64}", token):
            raise Denied("bootstrap credential is invalid")
    return value


def token_hash(value):
    return hashlib.sha256(value.encode()).hexdigest()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Authority:
    def __init__(self, endpoint, token):
        self.url = endpoint.rstrip("/") + "/internal/cloud-dispatch/authorize"
        self.token = token
        self.opener = urllib.request.build_opener(NoRedirect())

    def authorize(self, command):
        outgoing = urllib.request.Request(
            self.url,
            data=json.dumps(command).encode(),
            headers={"Authorization": "Bearer " + self.token, "Content-Type": "application/json"},
            method="POST",
        )
        try:
            with self.opener.open(outgoing, timeout=10) as response:
                raw = response.read(16385)
                if len(raw) > 16384:
                    raise Pending("authorization response is too large")
                result = json.loads(raw)
        except urllib.error.HTTPError as error:
            if error.code in (400, 401, 403, 404, 409, 422):
                raise Denied("attempt dispatch is not authorized") from None
            raise Pending("dispatch authority is unavailable") from None
        except (OSError, ValueError) as error:
            raise Pending("dispatch authority is unavailable") from error
        expected = {"action", "attempt_id"}
        if command["action"] == "start":
            expected |= {"image", "deadline_unix"}
        if not isinstance(result, dict) or set(result) != expected:
            raise Pending("dispatch authority returned invalid fields")
        if result["action"] != command["action"] or result["attempt_id"] != command["attempt_id"]:
            raise Pending("dispatch authority returned another attempt")
        if command["action"] == "start":
            image = result["image"]
            if not isinstance(image, str) or not re.fullmatch(r"[^\s@]+@sha256:[0-9a-f]{64}", image):
                raise Pending("dispatch authority returned an unpinned image")
            if type(result["deadline_unix"]) is not int or result["deadline_unix"] <= 0:
                raise Pending("dispatch authority returned an invalid deadline")
        return result
