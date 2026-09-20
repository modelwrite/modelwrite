# mw-server

The modelwrite model service: repository and version control, the fidelity gate, merge, and lock enforcement over Open Knowledge Format (OKF) documents, exposed as an HTTP API with a workbench UI. Part of the [modelwrite](https://github.com/modelwrite/modelwrite) MBSE engine (AGPL-3.0-or-later).

The mw-cli crate builds its offline mode on this crate's SQLite store (mw-server with default-features = false).
