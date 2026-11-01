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
At completion of that milestone, no crate had been published.

## Release automation alignment — 2026-10-19

- Set the minimum/pinned Rust version to 1.98.1; the 1.85
  checks above record the earlier implementation milestone.
- Add mise tasks, release scripts and their isolated tests,
  git-cliff configuration, and reusable lint/test/release workflows.
- Move lint scheduling to hk, with parallel checks, with a fast file-filtered path
  and a full profile for Clippy/rustdoc. Keep release-script tests in the common
  test gate. Pin hk, Python, and Docker CLI alongside the other mise tools.
- Retain the Linux, Docker, socket, and systemd test gates.
- Add package metadata, archive contents, changelog, and a first-release guide.
- Validate packaging without uploading; release tags and publication remain a
  separate action after reviewing the prepared candidate.

## First release quality gate

- Enable explicit import/formatting style lints and the cognitive complexity,
  function length, argument count, nesting, type complexity, and boolean parameter
  limits in `clippy.toml`. Treat their warnings as errors in hk and CI.
- Split the large integration fixture into resource-specific helpers and import
  types/modules directly. No lint suppressions are required.
- Include the lint configuration in the archive so the checks are reproducible
  from the published source.

## Deferred beyond this milestone

SSH and Windows targets, non-Docker transports, sudo, container lifecycle APIs,
firewall/block-device/configuration-management modules, a standalone spec runner,
and guaranteed remote command cancellation. Existing container lifecycle crates
can be combined with this library. Each deferred area needs its own scoped plan.

## 0.2.0: bounded system checks

A hung observation must return a typed timeout error so a caller can report it
and continue with independent checks. This extends Python Testinfra's local and
Docker execution capabilities; it is not a parity claim.

- Keep the synchronous API. Default to 30 seconds per underlying command, with
  validated host overrides and constructor overrides for Docker discovery.
- Bound local output collection and Docker negotiation, exec creation, streaming,
  and inspection. A multi-command resource method has a separate budget per
  command. Docker context discovery and its initial probe have separate budgets.
- Return `Error::Timeout`, distinct from absence, nonzero status, and transport
  failure. Discard partial output; do not put payloads or arguments in this error.
- Request termination of the local direct child and allow up to one extra second
  for reaping. Close inherited pipes rather than waiting for descendants. Do not
  promise descendant termination or cancellation of Docker's remote command.
- Let the caller or test runner decide continuation, reporting and fixture cleanup.
  Keep blocking-worker adapters in async callers. Stdin and a public async API
  remain separate proposed work, not prerequisites for deadlines.
- Reuse Tokio process and time support instead of custom process polling and
  pipe-reader threads. Enabling its process feature adds signal-hook-registry
  to the lockfile; no new direct dependency is required.
- Gate: formatting and common checks; local timeout and subsequent-success tests,
  inherited-pipe coverage, async-caller coverage, invalid budgets, and Docker
  blocked-resource-read and subsequent-success tests. Record platform limits.

Validation on 2026-11-01:

- `mise run fmt` and `mise run check`: passed (Clippy, rustdoc, unit/doc tests,
  lint tools and release-script tests).
- 14 unit tests passed on macOS and in a disposable Linux container. The Linux
  resource tests passed, including a FIFO read that times out before a successful
  independent file check. Local Linux infrastructure and socket checks passed.
- `mise run test:docker`: passed against Alpine, including a blocked FIFO read,
  a hung command and successful subsequent checks on the same host.
- `cargo package --allow-dirty --locked`: built and verified the 0.2.0 archive.
- No hosted workflow, live systemd test or full distro matrix was run for this
  change. The candidate is not published. Congru adoption and stdin remain open;
  this does not complete the original combined R2 deliverable.


Consumer smoke validation on 2026-11-01:

- Temporarily selected this local 0.2.0 candidate in Congru and ran two supervised
  acceptance tests with existing Debian/systemd images. Both passed (4.930 seconds
  of test execution; 19.712 seconds including compilation and test startup).
- The smoke test included a temporary FIFO probe: a one-second resource-read
  deadline returned `Error::Timeout`, then file and service checks succeeded on
  the same host. Operator access and the existing stdin helper also passed.
- The diversion lifecycle covered check, apply, unchanged apply and retirement.
  The supervisor reported successful cleanup of both projects.
- Restored the consumer manifest, lockfile and temporary probe after the run.
  This is focused compatibility evidence, not full-suite validation or permanent
  adoption. Existing images were reused; no new engine image was built.


## Next: command stdin (first 0.3.0 slice)

- Add `Host::exec_with_input` and `Host::run_with_input` for binary payloads.
  Existing methods send empty input. Close stdin after the payload, including
  empty input; preserve literal argument execution and explicit shell execution.
- Transfer input, stdout and stderr concurrently under one command timeout.
  A command may exit without reading all input. Treat a broken stdin pipe as an
  early close, while preserving the command's output and status. Other I/O and
  Docker inspection failures remain errors. A timeout discards partial output.
- Reuse Tokio's asynchronous pipes and Bollard's attached input stream and
  shutdown support. No custom protocol or new dependency is needed.
- Validate binary input, EOF-dependent completion, large simultaneous streams,
  early exits, blocked stdin and use from an existing runtime. Run the same
  exchange cases on local Linux and Docker where applicable.

### Consumer guarantee comparison

The current consumer helper captures literal `docker exec -i` commands with a
90-second budget (or a caller override), binary stdout/stderr and a 4 MiB limit
per output stream. It tolerates broken stdin pipes and kills the local process
group on failure; remote processes can survive until container teardown.

Testinfra supports literal commands, configurable deadlines and binary stdout.
This slice adds stdin but keeps the existing lossy UTF-8 stderr string and
unbounded output buffering. Before migrating that helper, add bounded capture
and lossless stderr or explicitly approve those differences. Local process-group
cleanup also differs: testinfra requests direct-child termination only. Docker
execution uses the native API and has no local Docker CLI process to kill.
Transcript storage, secret redaction and expected-status assertions stay with
the caller. Supervisor/local-tool execution and complete removal of the consumer's
process crate are outside this slice. Do not migrate the helper yet.

Implementation evidence:

- `exec_with_input` and `run_with_input` are implemented using the existing
  Tokio/Bollard dependencies. Cargo.toml and Cargo.lock are unchanged.
- The first Docker run found that an exited command can close its output while
  the input write remains pending. The implementation now drops that write on
  output EOF and inspects the exec status before returning a result. Early-exit
  tests cover zero, partial and explicitly closed input with an 8 MiB payload.
- The exchange suite passes on local Linux and Docker Alpine 3, Debian bookworm
  slim and Fedora 44. It covers binary payloads and literal arguments, empty-input
  EOF, 1 MiB transfers on all three streams, nonzero status, blocked writes, a
  command that consumes input then hangs, and calls from a Tokio runtime.
- Existing resource tests pass across the same Docker matrix. All local Linux
  unit, resource and documentation tests pass. `test:docker` now includes the
  stdin suite so the existing hosted Docker matrix will exercise it.
- `mise run fmt` and the full `mise run check` gate pass, including Clippy,
  API documentation and release-script tests.
- No new live-systemd test, hosted run, consumer migration or publication was
  performed. Output limits and lossless stderr remain the next bounded slice
  before replacing the consumer command helper.


Debian validation target update:

- Use `debian:trixie-slim` in the hosted matrix and current development/release
  instructions. The earlier bookworm run above remains historical evidence.
- `TESTINFRA_TEST_IMAGE=debian:trixie-slim mise run test:docker` passes both
  resource checks (7.57 seconds) and stdin checks (2.87 seconds). Workflow lint
  passes after the matrix change.
