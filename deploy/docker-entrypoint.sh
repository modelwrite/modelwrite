#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Entrypoint for the modelwrite service image.
#
# mw-server runs in OPEN mode when neither MW_AUTH_TOKEN nor MW_AUTH_JWKS is
# configured, which accepts every request as an anonymous administrator. That is
# correct for a pilot on a laptop and wrong for anything exposed, so this
# entrypoint REFUSES to start unless authentication is configured OR the operator
# sets MW_ALLOW_OPEN=yes explicitly. A production service must not be able to run
# unauthenticated by accident.
set -eu

# Blank when empty or whitespace-only, matching the server's trim().is_empty().
# This keeps the gate consistent with how mw-server itself decides there is no
# credential: a whitespace-only MW_AUTH_TOKEN is NOT a token.
is_blank() {
    case "$1" in
        *[![:space:]]*) return 1 ;;  # has a non-whitespace character
        *) return 0 ;;               # empty or whitespace only
    esac
}

# The server accepts the opt-in case-insensitively, so this guard must too: an operator
# who writes MW_ALLOW_OPEN=YES should not have the container refuse while the server
# would have accepted it. One rule, applied in both places.
mw_allow_open=$(printf '%s' "${MW_ALLOW_OPEN:-}" | tr '[:upper:]' '[:lower:]')

if is_blank "${MW_AUTH_TOKEN:-}" && is_blank "${MW_AUTH_JWKS:-}"; then
    if [ "$mw_allow_open" != "yes" ]; then
        cat >&2 <<'EOF'
REFUSING TO START: no authentication is configured.
mw-server runs in OPEN mode with no credential, which accepts every request as
an anonymous administrator. Set MW_AUTH_TOKEN to require a shared bearer token,
or MW_AUTH_JWKS to the path of a JWKS file to require signed JWTs. To run open on
purpose (a laptop pilot only), set MW_ALLOW_OPEN=yes.
EOF
        exit 1
    fi
fi

exec /usr/local/bin/mw-server "$@"
