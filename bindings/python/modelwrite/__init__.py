# SPDX-License-Identifier: AGPL-3.0-or-later
"""modelwrite: a thin read-only Python client for the modelwrite analytics REST surface.

Typical use::

    from modelwrite import analytics

    tables = analytics.tables("coffee")                 # all nine tables
    elements = analytics.tables("coffee", table="elements")
    frame = elements.to_pandas()                        # needs pandas
    metrics = analytics.metrics("coffee")
    coverage = analytics.trend("coffee", "coverage.covered", "main")
"""

from . import analytics
from ._client import Client, DEFAULT_BASE_URL
from ._schema import SCHEMA_VERSION
from ._table import Table, Tables
from .errors import (
    BadRequest,
    Forbidden,
    HTTPError,
    ModelwriteError,
    NotAuthorized,
    NotFound,
    ServiceUnavailable,
)

__version__ = "0.1.0"

__all__ = [
    "analytics",
    "Client",
    "DEFAULT_BASE_URL",
    "SCHEMA_VERSION",
    "Table",
    "Tables",
    "ModelwriteError",
    "HTTPError",
    "BadRequest",
    "NotAuthorized",
    "Forbidden",
    "NotFound",
    "ServiceUnavailable",
    "__version__",
]
