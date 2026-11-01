# Changelog

## 0.2.0 — 2026-11-01

- Bound commands and resource-read commands with a configurable 30-second
  default timeout. Report `Error::Timeout` so callers can continue other checks.
- Add Docker constructor timeout options, including context discovery and the
  initial Linux probe. Document direct-child and remote cleanup limits.
- Reuse Tokio process support; stdin and a public async API remain deferred.

## 0.1.0 — 2026-10-19

- Initial Testinfra-inspired Rust library for local Linux and existing Docker containers.
- File, package, user/group, process, systemd service, system information,
  environment, sysctl, mount, and socket inspection.
- Literal command execution through std::process and Bollard, preserving binary stdout.
- Native Linux, Alpine/Debian/Fedora, live systemd, and Docker socket tests.
- Rust 1.98.1, shared mise checks, and tested release automation reused from tickerapp-rs.
- Parallel, file-filtered hk linting for fast feedback, with development tools pinned by mise.
- Enforced Clippy style and complexity limits, with resource-specific integration test helpers.
