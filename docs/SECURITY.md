# Security — As-Built Controls & Target Gaps

> **As-built** facts first. Target deep-dive (OWASP course): [`bn/18-security-architecture.md`](./bn/18-security-architecture.md).  
> Readiness: [`PRODUCTION_READINESS.md`](./PRODUCTION_READINESS.md).

**Audit date:** 2026-07-16

---

## Current posture (honest)

The platform is **not** hardened for production multi-tenant monitoring data.

| Layer | Today |
|-------|--------|
| Web | Demo auth via `localStorage` — anyone can spoof role/workspace in the browser |
| API | OpenIddict + ABP Identity present; **no** product Activity APIs to abuse yet |
| Agents | Store credentials/config locally; call missing login routes; offline SQLite queue |
| Secrets | Dev `CertificatePassPhrase` / `StringEncryption:DefaultPassPhrase` in tracked `appsettings.json` |
| Transport | HTTPS expected for Host; agents must trust local certs |

---

## Threat model (monitoring SaaS)

| Threat | Impact | As-built mitigation | Needed |
|--------|--------|---------------------|--------|
| Cross-tenant data read | Critical | ABP tenant infra exists; **no** activity tables yet | Global query filters + tests on every product query |
| Privilege escalation (admin→owner) | High | UI invite bug possible | Server permissions; never trust client role |
| Agent credential theft | High | Partial (local config) | Device tokens, short TTL, refresh rotation, OS secret store |
| Screenshot leakage | High | N/A (storage not built) | Private bucket, presigned GET, no public ACL |
| Keystroke content collection | Critical (legal) | Agents designed for **counts only** | Code review + policy tests; never log key values |
| Token replay / XSS on dashboard | High | Mock session only | HttpOnly cookies or careful bearer storage; CSP |
| Supply chain / unsigned agents | Medium | No signed update channel | Signed installers + update manifests |
| Insider Host abuse | High | Host UI is mock | Real Host RBAC + audit log for impersonation |

---

## Controls that already exist

- ABP **Permission Management** / Identity / OpenIddict modules (template).  
- Serilog audit trail capability via ABP Audit Logging tables (after migrate).  
- Health endpoint for liveness (do not expose sensitive diagnostics publicly without auth).  
- Agent design: keyboard/mouse **counts**, not content (must remain non-negotiable).

---

## Controls that do **not** exist yet

- Product RBAC on Activities / Projects / Monitor.  
- Refresh-token rotation + reuse detection (course target).  
- Rate limits on ingest.  
- Encrypted-at-rest screenshot store.  
- Security headers / CSP on Next production deploy.  
- CI secret scanning / dependency audit gate.  
- Formal penetration test / GDPR retention enforcement.

---

## Hard rules for implementers

1. **Never** trust `role` or `tenantId` from the SPA body — resolve from authenticated principal.  
2. **Every** activity query filters by tenant (ABP data filter on).  
3. Agents: **counts only**; no clipboard/key content.  
4. Screenshots: private storage; blur policy server-enforced where promised.  
5. Host impersonation: time-boxed, audited, reason required.  
6. Remove default passphrases from git before any shared deployment.

---

## Auth alignment debt

| Client | Expects | Server has |
|--------|---------|------------|
| Web demo | `localStorage` user | Nothing wired |
| Agents | `POST /api/app/account/login` + bearer | OpenIddict token endpoints (different contract) |
| Future SPA | ABP / OIDC | OpenIddict — prefer standard OIDC over custom `/api/app/account/login` |

**Decision needed:** either implement thin agent login wrapping OpenIddict, or change agents to password/device flow against OpenIddict. Document the chosen contract in as-built when done.

---

## Reporting

Security issues: follow the repo’s preferred channel (GitHub Security Advisories if enabled; otherwise private contact in README). Do not file public issues with exploit detail for unpatched production deploys.
