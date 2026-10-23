# 10 — Risks

## Discord Terms of Service ⚠️
Third-party clients and client mods (including Vesktop/Vencord) violate Discord's ToS. Discord rarely bans people just for using one, but automated-looking behaviour does get accounts flagged. Mitigations:
- Behave like a normal client: respect rate limits, don't make unprompted bulk API calls (e.g. fetching every member of every guild), and send typing/ack events the way the official client does.
- No self-bot features: no auto-replies, scheduled messages, or mass actions.
- Put a clear warning on the login screen and in the README: **"Use at your own risk."**

## API drift
The user API is undocumented and changes without notice (new READY fields, member-list ops, upload flows, client header changes).
- Deserialize leniently: tolerate unknown fields and enum values (`#[serde(other)]`).
- Keep protocol constants in one place.
- Record gateway fixtures and diff them when things break.

## Security
- The token is equal to full account access. Keep it only in the keyring and redact it from logs and crash reports.
- Treat everything from the network as untrusted. Fuzz the markdown parser and the image decoders.
- Open links through the system handler with a confirm dialog for unknown domains.

## Scope creep
Discord has hundreds of features. Stick to the roadmap. Each milestone must be usable before starting the next one.
