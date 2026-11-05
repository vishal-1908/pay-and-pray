# Security Policy

## Reporting a vulnerability

If you find a security issue in Pay and Pray, **please don't open a public issue.** Instead:

1. Open a **private security advisory** at [github.com/vishal-1908/pay-and-pray/security/advisories/new](https://github.com/vishal-1908/pay-and-pray/security/advisories/new).
2. Include: what the issue is, how to reproduce it, what an attacker could do, and (if you have one) a suggested fix.
3. I'll acknowledge as soon as I can — best-effort, no committed SLA. Pay and Pray is maintained solo, so please be patient.

If GitHub's private advisory flow isn't available to you, fall back to emailing the address listed on [@vishal-1908](https://github.com/vishal-1908)'s GitHub profile.

## Supported versions

Pay and Pray is in active development and only the latest tagged release is supported. Fixes will land in a new release, not as a back-port.

| Version    | Supported |
| ---------- | --------- |
| latest tag | ✅        |
| older tags | ❌        |

## Threat model

Pay and Pray is a personal desktop app that polls two AI vendor APIs and stores nothing beyond your local machine. The realistic risks:

- **Stolen API token / cookie.** Mitigated by storing credentials only in the OS keychain (Windows Credential Manager / macOS Keychain / Linux Secret Service). Never logged, never written elsewhere, never sent off-machine. The Claude `sessionKey` is a full login for the account (not a read-only usage key), so the sign-in webview's session data is wiped as soon as the key is captured; the keychain holds the only copy. The default "Use Claude Code login" path avoids the session cookie entirely: the app reads Claude Code's own scoped OAuth token live from its keychain item (only after you opt in, never copied) and falls back to the cookie only if that fails. Use the app only on a machine you trust and sign out when done.
- **Compromised binary.** Same risk as any desktop app: if you install a malicious build, it can do whatever your user account can do. Always install from [github.com/vishal-1908/pay-and-pray/releases](https://github.com/vishal-1908/pay-and-pray/releases) (the only official source). Code signing is on the roadmap (Phase 5) but not yet shipped — verify checksums if you're cautious.
- **Other user-level malware on the same machine** can read OS keychain items requested by name. This is an OS limitation, not specific to Pay and Pray; the entire desktop app ecosystem shares it.

## What Pay and Pray doesn't do (by design)

- ❌ Send any telemetry, analytics, or usage data off-machine.
- ❌ Log credential values (verified — only error objects are logged, never tokens or cookies).
- ❌ Store credentials anywhere except the OS keychain.
- ❌ Bundle a client secret (the device-flow / PAT paths require none).
- ❌ Accept inbound network connections.

See also: [FAQ.md § Privacy & data](FAQ.md#privacy--data).
