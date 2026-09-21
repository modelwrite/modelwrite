These keys are TEST ONLY and must never be used to sign real tokens.

This directory holds a committed RSA-2048 key pair (plus a second private key) used
only by the JWT authentication tests under `server/tests/auth.rs`. They are committed
on purpose: tests need no network access to an identity provider and no in-test key
generation, so the suite is deterministic and works in an air-gapped checkout.

- `rs256_private.pem` - the signing key the tests use; it is NOT a secret.
- `rs256_other_private.pem` - a different signing key proving that a token signed by
  the wrong key is refused.
- `jwks.json` - the public half only (`kid` `test-key`, modulus `n`, exponent
  `e`), in JWK-set form, which is what an operator would mount at `MW_AUTH_JWKS`.

Because the private keys are published in this repository, they provide no security at
all: never sign a production token with them, and never reuse them in a real identity
provider.

## SysML v2 fixture

`Drone_BaseArchitecture.sysml` is a REAL community model, copied byte-for-byte from
`sample/examples/sysml-v2/gfse-models/models/SE_Models/Drone_BaseArchitecture.sysml`,
committed here so the server's SysML v2 import test does not depend on the whole bundled
corpus.

Licence: BSD 3-Clause, Copyright (c) 2024, Gesellschaft für Systems Engineering e.V.
The full text is at `sample/examples/sysml-v2/gfse-models/LICENSE`. Redistribution
with the copyright notice, the conditions and the disclaimer retained is permitted
without additional approval; committing the model here (with the notice recorded) is
the redistribution the licence permits.
