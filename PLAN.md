# Linux and Docker implementation plan

Work through these slices in order. A slice is complete when its code, meaningful
tests, and documentation are updated and its listed checks pass. Search for a
suitable existing crate before implementing general-purpose functionality.

This milestone expands the existing library into a practical Linux/Docker core.
It does not promise all Python Testinfra modules or Python API compatibility.

## 1. Execution foundation — complete

- Execute literal argv directly through std::process and Bollard.
- Keep explicit shell scripts separate from program invocation.
- Cover invalid arguments, binary output, command failure, and Docker failure.
- Verify the declared Rust 1.85 minimum against the dependency lockfile.
- Gate: unit tests, Rust 1.85 check, Alpine Docker integration test.
- Evidence: all three checks passed. `exec` uses direct argv on both backends;
  `run_expect` provides explicit accepted-status checks.

## 2. Files and accounts — complete

- Add file UID/GID/size, symlink target, and executable checks.
- Return a typed missing-resource error when an account property is requested.
- Exercise tricky filenames, symlinks, and account lookups on Linux targets.
- Gate: unit tests and Docker resource tests.
- Evidence: unit and Alpine Docker tests passed, including newline-containing
  paths, stored symlink targets, executable bits, and typed missing accounts.
- Crate review: pwd-grp/nix query the runner's account database, so target-side
  getent and its small record adapter remain appropriate here.

## 3. Environment, sysctl, and mounts — complete

- Read the target environment without losing embedded newlines.
- Read sysctl values without writing kernel configuration.
- Parse the target mount table using an existing crate.
- Gate: fixture parsing tests and Linux/Docker integration checks.
- Evidence: unit fixtures and Alpine integration passed. procfs-core parses
  mountinfo; unescaper decodes its escaped paths. The inspection APIs only read
  target state. Native Linux coverage is included in the final validation slice.

## 4. Listening sockets — complete

- Check TCP/UDP IPv4/IPv6 and Unix sockets in the target network namespace.
- Reuse a crate to parse procfs rather than inventing address decoding.
- Validate present/absent listeners with real socket fixtures.
- Gate: parser and native Linux fixture tests, Docker namespace checks.
- Evidence: cross-endian parser fixtures, native Linux TCP/UDP/IPv6/Unix tests,
  and a Python socket fixture inside Docker passed. tempfile manages native
  fixture directories. Unix flag/path supplementation uses the existing regex
  crate because procfs-core does not expose those fields completely.

## 5. Services and release checks — implementation complete; live CI validation pending

- Verify active/inactive, enabled/disabled, missing, and unsupported service cases.
- Add an opt-in live systemd test and configure Linux CI to run it.
- Update the resource matrix, runnable example, crate rationale, and limitations.
- Gate: format, Clippy, docs, minimum Rust, native Linux, and the
  Alpine/Debian/Fedora Docker matrix. Record any environment-dependent checks
  that cannot be run locally rather than claiming they passed.
- Evidence: formatting, Clippy, documentation with warnings denied, Rust 1.85.1
  checks, 10 unit tests, doc tests, and the full native Linux test suite passed.
  Resource tests passed on Alpine, Debian, and Fedora. The Docker socket fixture
  also passed. The systemd fixture shell script passed its syntax check.
- Service state fixtures cover positive, disabled/linked/masked, inactive/failed,
  and operational-error outcomes. Docker checks cover an unsupported manager.
- Live systemd validation has not run locally: this macOS workspace and its test
  containers do not have a running systemd manager. The opt-in fixture and CI
  step are implemented, with cleanup of only their own runtime units. A CI run
  must still verify the live manager paths, including missing units.

## Milestone result

All five slices were implemented in order, reusing crates for generic work.
The only outstanding validation is the live systemd test on a suitable host/CI.
No crate has been published and no external CI run has been triggered.

## Deferred beyond this milestone

SSH and Windows targets, non-Docker transports, sudo, container lifecycle APIs,
firewall/block-device/configuration-management modules, a standalone spec runner,
and command cancellation/timeouts. Existing container lifecycle crates can be
combined with this library. Each deferred area needs its own scoped plan.
