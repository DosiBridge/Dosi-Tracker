# Security Policy

Dosi-Tracker handles workspace activity data for real teams. We take reports
of security vulnerabilities seriously and ask that you report them
responsibly, as described below.

For the as-built threat model, current controls and known gaps, see
[`docs/SECURITY.md`](./docs/SECURITY.md). This file is about **reporting a
vulnerability** you have found; that one is about what the system does today.

## Supported versions

Only the latest published release of each component (backend, frontend,
Windows/macOS/Linux agents — see [GitHub Releases](https://github.com/DosiBridge/Dosi-Tracker/releases))
receives security fixes. Self-hosters should stay on the latest release;
[`docs/SELF_HOSTING.md`](./docs/SELF_HOSTING.md) covers upgrades.

## Reporting a vulnerability (DEVOPS-13)

**Please do not open a public GitHub issue for a security vulnerability.**
A public issue or pull request can be scanned and exploited before a fix
ships, putting every self-hosted instance and SaaS tenant at risk.

1. **Preferred: GitHub Security Advisories.** Open
   [github.com/DosiBridge/Dosi-Tracker/security/advisories/new](https://github.com/DosiBridge/Dosi-Tracker/security/advisories/new)
   ("Report a vulnerability" under the repository's Security tab). This opens
   a private advisory visible only to maintainers, with its own thread for
   follow-up questions and coordinated disclosure.
2. **If that tab is unavailable to you**, open a
   [private security report](https://github.com/DosiBridge/Dosi-Tracker/security/policy)
   or contact a maintainer directly through their GitHub profile and ask them
   to open an advisory on your behalf; do not post exploit details in that
   first message.

Please include, as far as you can:

- The affected component (backend, frontend, a specific desktop agent, CI/CD,
  or the Docker images) and version or commit.
- Steps to reproduce, or a proof of concept — enough for us to confirm the
  issue without guessing.
- The impact you believe it has (e.g. cross-tenant data access, privilege
  escalation, credential exposure, remote code execution).
- Any suggested fix or mitigation, if you have one.

Never include real user data, real credentials, or a live victim workspace in
a report. Reproduce against a disposable, self-hosted instance (see
[`docs/SELF_HOSTING.md`](./docs/SELF_HOSTING.md)) or synthetic/demo data.

## What to expect

- **Acknowledgement** within 3 business days of a report through the channels
  above.
- **Triage and severity assessment** within 10 business days, including
  whether and how we can reproduce it.
- **A fix or mitigation plan** communicated to you before any public
  disclosure. Timelines depend on severity and complexity; we will keep you
  updated rather than go silent.
- **Credit**, in the published GitHub Security Advisory and release notes, if
  you want it — tell us how you'd like to be named, or ask to stay anonymous.

We ask reporters to give us a reasonable window to ship a fix before any
public disclosure of the vulnerability or a proof of concept, and we will
coordinate a disclosure date with you once a fix is ready.

## Scope

In scope: the backend API (`backend/`), the web dashboard (`frontend/`), the
desktop agents (`clients/windows`, `clients/macos`, `clients/linux`), the
Docker images and Compose stack (`docker-compose*.yml`, `backend/**/Dockerfile*`,
`frontend/Dockerfile`), and this repository's CI/CD pipelines
(`.github/workflows/`).

Out of scope: automated vulnerability-scanner reports with no demonstrated
impact, issues that require physical access to an already-compromised device
or an already-stolen credential, and social engineering against maintainers
or users. Reports about the privacy invariants (keyboard/mouse **counts**
only, never content; no root/sudo capture; camera off by default) that show an
agent capturing more than it discloses are very much in scope — please report
those the same way.

## Safe harbor

We will not pursue legal action against good-faith security research that:

- Stays within the scope above,
- Avoids privacy violations, data destruction, and service disruption,
- Uses a disposable self-hosted instance or your own account/workspace rather
  than another user's data, and
- Reports the issue through the channels above before any public disclosure.
