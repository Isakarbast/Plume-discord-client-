# ADR 0002 — QR remote auth and token login only

**Status:** Accepted (planning)

**Decision:** Support QR remote-auth login and pasting a token. Don't implement email/password login.

**Why:** Password login requires solving captchas and handling MFA, and it's the most likely path to get accounts flagged. With QR login, the user's password never touches our app.

**Update (2026-10):** Discord sometimes answers the QR ticket exchange with a captcha. Plume does not solve or evade captchas. It reports `CaptchaRequired`, and the user falls back to pasting a token once, which is verified against `/users/@me` and saved to the keyring.
