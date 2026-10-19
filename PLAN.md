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

## 5. Services and release checks — complete

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
- Live systemd validation, including missing units, passed in
  [the first CI run](https://github.com/gilesw/testinfra-rs/actions/runs/39282291601).
  This closes the check that could not run on the macOS workspace.

## Milestone result

All five slices were implemented in order, reusing crates for generic work.
The first GitHub Actions run passed all jobs, including live systemd validation.
No crate has been published.

## Release automation alignment — 2026-10-19

- Align the minimum/pinned Rust version with tickerapp-rs at 1.98.1; the 1.85
  checks above record the earlier implementation milestone.
- Reuse tickerapp-rs mise tasks, release scripts and their isolated tests,
  git-cliff configuration, and reusable lint/test/release workflows.
- Move lint scheduling to hk, following congru, with a fast file-filtered path
  and a full profile for Clippy/rustdoc. Keep release-script tests in the common
  test gate. Pin hk, Python, and Docker CLI alongside the other mise tools.
- Retain the Linux, Docker, socket, and systemd test gates.
- Add package metadata, archive contents, changelog, and a first-release guide.
- Validate packaging without uploading; release tags and publication remain a
  separate action after reviewing the prepared candidate.

## Deferred beyond this milestone

SSH and Windows targets, non-Docker transports, sudo, container lifecycle APIs,
firewall/block-device/configuration-management modules, a standalone spec runner,
and command cancellation/timeouts. Existing container lifecycle crates can be
combined with this library. Each deferred area needs its own scoped plan.
