/* SPDX-License-Identifier: AGPL-3.0-or-later */
#ifndef MODELWRITE_H
#define MODELWRITE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Version string; never free it. */
const char *modelwrite_version(void);

/* Validate one OKF JSON document; returns a JSON report string.
   The caller owns the result and must free it with modelwrite_free_string. */
char *modelwrite_validate(const unsigned char *json, size_t len);

/* Run the round-trip fidelity gate; returns the JSON evidence string.
   The caller owns the result and must free it with modelwrite_free_string. */
char *modelwrite_gate(const unsigned char *reference, size_t reference_len,
                      const unsigned char *candidate, size_t candidate_len);

/* Free a string returned by this library. */
void modelwrite_free_string(char *ptr);

#ifdef __cplusplus
}
#endif

#endif
