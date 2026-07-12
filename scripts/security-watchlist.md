# Security watchlist

Treat these as review classes, not automatic findings. Confirmation requires exact code path, attacker-controlled input, reachable preconditions, plausible impact, and reproducible proof or test.

- command or argument injection
- path traversal, arbitrary file read/write/delete, symlink or hardlink race
- temp-file race, TOCTOU, canonicalization or import-path confusion
- SSRF, DNS rebinding, open redirect
- XSS, HTML/JS/template/expression/CSS injection
- unsafe deserialization, prototype pollution, object injection
- log/header/CRLF injection, request smuggling
- authn/authz bypass, IDOR, tenant escape, privilege escalation
- CSRF, CORS misconfiguration, replay, session or MFA bypass
- secret leakage, hardcoded credentials, verbose error or source-map disclosure
- weak/predictable randomness, weak hashing, collision abuse, cache poisoning
- resource, CPU, memory, thread, stack, or recursion exhaustion
- deadlock, livelock, race, data race
- integer overflow/underflow, out-of-bounds access, use-after-free, double free
- null dereference, panic/unwrap/expect on untrusted input, unchecked indexing
- unsafe code or FFI, buffer overflow
- environment/process path poisoning, debug exposure, build-artifact overwrite
