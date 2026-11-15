# Publication review

This review covers the source prepared for publication on 2026-11-15. It records
specific checks and their limits rather than a guarantee that the software has
no defects.

## Repository boundaries

- `daily` contains the Electron app, Rust workspace and shared TypeScript plugin SDK.
- `daily-agent-core` is independent of HTTP, desktop state, plugins and SQLite.
- `daily-agent-providers` depends on Core and owns model transports. Subscription
  login support is behind its optional `login` feature.
- `daily-desktop-runtime` owns settings, persistence, plugin validation and task
  storage. `daily-core` remains a compatibility re-export.
- Writer and Timeline are separate repositories using the host SDK. Their builds
  verify that ES modules contain no unresolved package imports, CommonJS require
  or Node environment access.

## Content and credentials

The Daily Git history and the three repositories' candidate source files were
scanned for private keys, common API/GitHub/cloud token formats, JWTs, literal
secrets and personal home-directory paths. The identified secret-like strings
were test fixtures; the historical home-directory example is a generic `/Users/me`.
No real credential was identified by these checks.

Runtime `.daily`/legacy `.daliy` directories, local settings, environment files,
manuscripts, scratch files, dependencies and build output are excluded from
commits. All npm lockfiles use the public npm registry. This scan cannot detect
every possible secret format or establish ownership of every text string.

Writer's six locally collected third-party sample passages have no redistribution
license recorded. They remain local and ignored, and their imports have been
removed so they are absent from production bundles. The five samples described
in the source as original remain available. User-imported style samples and
manuscripts are runtime data and are not distributed.

The repositories use MIT licensing for project source. Third-party dependencies
retain their respective licenses. Subscription adapters are documented as
unofficial; no third-party OAuth client secret is distributed.

## Windows corrections

- Resolve native `.exe` names in development and packaged sidecar paths.
- Compare canonical workspace paths using the same Node resolver, including
  Windows extended-length paths.
- Run real sidecar tests on every CI platform, including spaces and Chinese
  characters in workspace paths.
- Reject case variants of `.daily`, and on Windows reject alternate data streams
  and trailing-dot/space filename aliases at the shared sandbox boundary.
- Close temporary Rust files before replacement and preserve existing atomic-write
  regression coverage.
- Use LF checkout rules and an OS-independent Vitest directory argument.
- Package a native x64 Windows sidecar into an unsigned NSIS installer and test
  the shipped sidecar in CI.

## Validation

Local validation includes the complete app test/build command, strict workspace
Clippy, plugin SDK TypeScript checks, both plugin test/build commands, a real
standalone Core consumer and a self-contained SDK source archive. Known npm
dependency vulnerabilities in the plugins' old build/test tools were addressed
by upgrading those tools and their locked transitive dependencies. The final npm audits for all three repositories and the RustSec audit of `Cargo.lock` reported zero known vulnerabilities.

GitHub Actions is the source of truth for the current native Windows/Linux CI
status. Workflows use standard hosted runners, read-only repository permissions,
bounded timeouts and seven-day artifact retention. No production provider keys
are required. Public-repository standard runner compute is free under GitHub's
published policy; storage and other optional paid services have separate limits.

Live provider integrations and interactive Windows desktop behavior require
separate acceptance checks. Antigravity sign-in remains macOS-only. See the
[Windows guide](windows.md) for setup and the manual acceptance sequence.
