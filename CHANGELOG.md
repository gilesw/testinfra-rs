# Changelog

## 0.1.0 — 2026-10-19

- Initial Testinfra-inspired Rust library for local Linux and existing Docker containers.
- File, package, user/group, process, systemd service, system information,
  environment, sysctl, mount, and socket inspection.
- Literal command execution through std::process and Bollard, preserving binary stdout.
- Native Linux, Alpine/Debian/Fedora, live systemd, and Docker socket tests.
- Rust 1.98.1, shared mise checks, and tested release automation reused from tickerapp-rs.
- Parallel, file-filtered hk linting for fast feedback, with development tools pinned by mise.
- Enforced Clippy style and complexity limits, with resource-specific integration test helpers.
