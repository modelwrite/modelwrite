# Registered trials: the two-tier trial design

**Status:** design, approved framing, awaiting owner review of this spec.
**Date:** 2026-09-21

## 12. Email consent - the two kinds, and the law

The registration flow sends email, so the two kinds must not be confused:

1. **The login code is TRANSACTIONAL email.** It delivers the service the person just requested. No marketing consent is required for it, and none is claimed. One email, one code, nothing else in it.
2. **Marketing email - updates, material about modelwrite - requires EXPLICIT OPT-IN.** At registration: an UNCHECKED box ("send me occasional updates about Modelwrite"), the choice recorded with a timestamp in the identity store, so it can be PROVEN later. Every marketing email carries a working unsubscribe link, and unsubscribing stops the email immediately. (The Australian Spam Act 2003 requires consent and a functional unsubscribe; the GDPR requires explicit consent for marketing.)
3. The consent state is visible on the account page and changeable at any time. Never bundled with the login - the code arrives regardless of the choice.
4. The sender must be a real address on the modelwrite.org domain with a contact address - the same contact the commercial brief already flags as missing.

**Delivery mechanism, stated honestly:** the mailer is PLUGGABLE. `MW_MAILER=smtp` selects a real SMTP transport, and the deployment emails the login code through Postmark: delivery is configured and proven there, and the credential the flow needs is in place. The console/file transports remain for offline tests only (`MW_MAILER=console` writes each code and its recipient to the operator log). The mailer still needs its SMTP settings - `MW_MAILER=smtp` with `MW_MAIL_SMTP_HOST`, `MW_MAIL_SMTP_USERNAME` or `MW_MAIL_SMTP_PASSWORD` unset FAILS the send and names the missing variable rather than logging the code. Real delivery does not change the flow.