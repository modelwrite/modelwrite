# SPDX-License-Identifier: AGPL-3.0-or-later
"""Exceptions raised by the modelwrite client.

The server reports every failure as an HTTP status plus a JSON body shaped like
{"error": "<message>"} (see server/src/error.rs). This module maps those statuses
onto a small, typed hierarchy so a caller can tell "auth is missing" (401) from
"project out of scope" (403) from "no such table" (404) without parsing the body.
"""


class ModelwriteError(Exception):
    """Base class for every error this package raises."""


class HTTPError(ModelwriteError):
    """An HTTP request did not succeed.

    status_code is the response status, message is the server error string
    (when present), and body is the raw response body for diagnostics.
    """

    def __init__(self, status_code, message, url=None, body=None):
        self.status_code = status_code
        self.message = message
        self.url = url
        self.body = body
        super().__init__(str(status_code) + ": " + message)


class BadRequest(HTTPError):
    """400 - a malformed request (unknown table, bad cursor, unknown filter)."""


class NotAuthorized(HTTPError):
    """401 - a bearer token is required and missing or invalid."""


class Forbidden(HTTPError):
    """403 - the caller lacks read permission or the project is out of scope."""


class NotFound(HTTPError):
    """404 - the project, commit or table does not exist."""


class ServiceUnavailable(HTTPError):
    """503 - the endpoint exists but is not enabled (OpenMetrics is off)."""


#: HTTP status -> the exception type the client raises for it.
STATUS_ERRORS = {
    400: BadRequest,
    401: NotAuthorized,
    403: Forbidden,
    404: NotFound,
    503: ServiceUnavailable,
}
