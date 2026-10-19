# testinfra-rs

A small, Testinfra-inspired Rust library for checking the actual configuration of
Linux hosts and running Linux Docker containers. Write ordinary `#[test]` functions
and run them with `cargo test`.

Requires Rust **1.98.1** or newer, matching tickerapp-rs. Development tooling is
pinned in `mise.toml`.

This is an initial implementation, not a complete API-compatible port of Python
[Testinfra](https://github.com/pytest-dev/pytest-testinfra). There is no Python or
target-side agent dependency.

## Use

The crate is not published. Add this checkout to your test project's `Cargo.toml`:

```toml
[dev-dependencies]
testinfra = { path = "../testinfra-rs" }
```

After the first release, use `testinfra = "0.1"` under `[dev-dependencies]`.

Create `tests/configuration.rs`:

```rust
use testinfra::{Host, Result};

#[test]
fn base_configuration() -> Result<()> {
    let host = Host::from_env()?;
    let passwd = host.file("/etc/passwd");
    assert!(passwd.is_file()?);
    assert!(passwd.contains(r"(?m)^root:")?);
    assert_eq!(passwd.user()?, "root");
    assert_eq!(passwd.mode()?, 0o644);
    assert_eq!(host.user("root").uid()?, 0);
    Ok(())
}
```

Run locally on Linux, or select an existing Docker container:

```sh
cargo test
TESTINFRA_HOST=docker://web cargo test
TESTINFRA_HOST=docker://root@web cargo test
```

Use `Host::local()`, `Host::docker("web")`, or
`Host::docker_as("web", "1000:1000")` directly when preferred. Docker targets work
from macOS as well as Linux. `Host::local()` rejects non-Linux targets.
The Docker daemon must be accessible to the caller. When the Docker CLI is
installed, its context inspection command resolves the selected daemon; otherwise
Bollard uses `DOCKER_HOST` or the default socket. Unix sockets and plain TCP are
supported; SSH and TLS endpoints are not currently enabled. Containers must
already be running and contain `sh` and `uname`.

An executable example is included:

```sh
cargo run --example check_host -- docker://web
```

## Resources

Every check returns a `Result`. An absent resource usually produces `false` or
`None`; failed execution, missing tools, and malformed output produce errors.
Reading properties or contents of an absent resource produces an error.
Resources are queried on demand without caching.

| API | Checks |
| --- | --- |
| `host.file(path)` | existence, regular file, directory, symlink/target, executable, bytes/text, regex/literal content, mode, UID/GID, owner/group, size |
| `host.package(name)` | installed state and version through dpkg, rpm, or apk |
| `host.user(name)` | existence, UID/GID, home, shell, primary and supplementary groups |
| `host.group(name)` | existence, GID, explicit supplementary members |
| `host.service(name)` | systemd running and enabled state |
| `host.process(pattern)` | PIDs and running state, matching the complete process name |
| `host.system_info()` | distribution ID, release, architecture, kernel |
| `host.environment()` | target environment, including empty and multiline values |
| `host.sysctl(key)` | existence and read-only string/typed kernel values |
| `host.mount_point(path)` / `host.mounts()` | mount presence, source, filesystem and options |
| `host.socket(specification)` | TCP/Unix stream listeners and unconnected bound UDP sockets |
| `host.run(script)` | explicit shell command with exit status, stdout bytes, stderr |
| `host.exec(program, args)` | program invoked directly with literal arguments |
| `host.run_expect(statuses, script)` | shell command requiring an accepted exit status |
| `host.check_output(script)` | UTF-8 stdout, requiring a successful exit |

For example:

```rust,no_run
# use testinfra::{Host, Result};
# fn checks(host: &Host) -> Result<()> {
assert!(host.package("nginx").is_installed()?);
assert!(host.service("nginx").is_running()?);
assert!(host.service("nginx").is_enabled()?);
assert!(host.process("nginx").is_running()?);
let result = host.exec("printf", &["%s", "literal $(text)"])?;
assert!(result.success());
assert_eq!(result.stdout_text()?, "literal $(text)");
# Ok(())
# }
```

## Semantics and current limits

- Targets need the commands used by each resource: GNU or BusyBox `stat` and `cat`
  for files, a supported package manager for packages, `id` for user groups,
  `pgrep` for processes, and `systemctl` plus running systemd for services.
  User/group lookups use `getent`, falling back to `/etc/passwd` and `/etc/group`.
- File metadata and content follow symlinks; `exists()` also recognizes dangling
  symlinks. `metadata().mode` includes special permission bits. File existence
  predicates follow shell `test` semantics, including permission restrictions.
  `link_target()` preserves the stored target, including trailing newlines.
- Missing user/group property lookups return `Error::NotFound`; `exists()` returns
  `false`. `readlink` is required for symlink target checks.
- File `contains()` uses Rust regex syntax, which differs from Python's regexes
  (for example, lookaround is unsupported). Use `(?m)` for line anchors.
- Process patterns use the target's POSIX extended regex syntax via `pgrep -x`.
  They match kernel process names, typically limited to 15 characters, rather than
  the full command line. Minimal images may need a process-tools package installed.
- Service checks support systemd only. A container without a running service
  manager returns `Error::Unsupported`. Checks never start or enable services.
  `is_enabled()` follows `systemctl is-enabled` exit-status semantics, including
  static and indirect units; it is not a guarantee the service starts at boot.
- Sysctl and mount checks read the target's `/proc` filesystem without changing
  kernel configuration. Sysctl keys use dotted components; components may contain
  letters, digits, underscores, and hyphens. Mount paths must be absolute.
- Environment inspection requires `env -0` and returns the inspection process's
  environment, including `LC_ALL=C`, rather than another process's environment.
  Values must be UTF-8; embedded equals signs and newlines are preserved.
- Socket specifications accept `tcp://80`, `tcp://127.0.0.1:80`,
  `tcp://[::1]:80`, `udp://*:53`, `unix:///run/example.sock`, or `unix://@name`.
  Internet addresses are literal IPs; names and CIDR ranges are unsupported.
  An explicit IP matches that exact binding; a port-only query matches any binding.
  UDP checks report unconnected bound sockets, rather than connected UDP clients.
  Checks read the target's procfs network tables, and use `od` to determine its
  byte order. Unix paths containing spaces are supported; paths containing newlines
  are rejected because the procfs table cannot represent them unambiguously.
- Package versions retain the manager's native formatting. Package names are exact
  names, not globs. Resource names are validated to prevent option interpretation.
- Stdout preserves binary bytes; text access requires UTF-8. Stderr is decoded
  with replacement characters for invalid UTF-8. Commands run with `LC_ALL=C`.
- Docker execution uses Bollard's Engine API client, output stream, and exec
  inspection. Target exit statuses remain separate from Docker API errors,
  including containers that stop between checks.
- Commands are synchronous, stdin is closed, and no execution timeout is imposed.
  Bound potentially blocking shell commands yourself, for example using Linux
  `timeout`. Output is buffered in memory, so avoid unbounded command output.
- `run()` and `check_output()` deliberately accept shell code. Use `exec()` for
  literal arguments, especially values from external input. Inspection is performed
  as the current local user or selected Docker user; there is no automatic sudo.
- SSH, Windows targets, other container runtimes, container lifecycle management,
  firewall/block-device resources, and a standalone test-spec CLI are outside the
  current scope. Use Rust's test runner for filtering, assertions, and reporting.

## Library choices

Reuse existing crates for general-purpose functionality:

- [bollard](https://docs.rs/bollard/) handles Docker's Engine API, stream framing,
  API version negotiation, and command exit status inspection.
- [tokio](https://docs.rs/tokio/) and [futures-util](https://docs.rs/futures-util/)
  drive Bollard behind the synchronous test API.
- [shlex](https://docs.rs/shlex/) quotes literal shell arguments.
- [etc-os-release](https://docs.rs/etc-os-release/) parses distribution data read
  from the target, rather than inspecting the machine running the tests.
- [thiserror](https://docs.rs/thiserror/) derives error implementations.
- [regex](https://docs.rs/regex/) provides file-content matching.
- [procfs-core](https://docs.rs/procfs-core/) parses target procfs data without
  reading the runner's Linux state. [unescaper](https://docs.rs/unescaper/) decodes
  escaped mount paths from mountinfo.

The procfs-core Unix socket record omits the accept flag and truncates paths at
whitespace. A small regex adapter supplements those two fields so a bound stream
socket is not incorrectly reported as listening and paths with spaces match exactly.

The systemd/libsystemd and zbus client crates query a D-Bus endpoint accessible
to the runner. Running systemctl inside the target keeps service inspection
consistent across local Linux and Docker without installing a Rust agent there.

Resource-specific Linux commands remain necessary to inspect the selected target.
Local-only inspection crates would inspect the test runner rather than the Docker
container. Tests use Docker's CLI to create and remove their isolated fixtures.

## Development

```sh
mise install
mise run check
mise run package:check
```

Mise owns the pinned tools, including hk, Python, and Docker CLI. A running Docker
daemon, Git, and the platform shell are prerequisites. The shared gate runs hk
lint checks, then Rust and release-script tests. CI uses the same tasks with
pinned Rust 1.98.1, plus the environment-specific tests below.

Lint configuration lives in `hk.pkl`, following the congru setup. hk runs
independent checks concurrently and filters them by relevant files:

```sh
mise run check:fast        # all fast lint checks, without test suites
mise run lint:changed      # fast lint checks selected by changed files
mise run lint              # all lint, including Clippy and rustdoc
mise run hooks:install     # optional local Git pre-commit hook through mise
mise run hooks:uninstall
```

The pre-commit hook uses working-tree contents and never fixes, stages, or stashes.
Clippy/rustdoc use hk's `slow` profile; the common gate and CI always enable it.
`mise run fmt` is the explicit command that rewrites Rust formatting.
Release-script tests remain mandatory in `mise run test` and `mise run check`.

Real Docker tests are opt-in. They create a uniquely named container, make fixture
files under `/tmp`, and remove the container on exit. Images are pulled by Docker
when missing. They exercise both successful checks and errors, including a stopped
container. The default image is Alpine:

```sh
cargo test --test infrastructure -- --ignored --nocapture
TESTINFRA_TEST_IMAGE=debian:bookworm-slim cargo test --test infrastructure -- --ignored
TESTINFRA_TEST_IMAGE=fedora:44 cargo test --test infrastructure -- --ignored
```

Equivalent tasks are `mise run test:docker` (honouring `TESTINFRA_TEST_IMAGE`),
`mise run test:sockets`, and `mise run test:systemd`.

CI runs native Linux tests and the Docker suite on Alpine, Debian, and Fedora.
The socket fixture uses `python:3.13-alpine` to open TCP, UDP, and Unix listeners;
Python is only needed inside that test image, not by the library.

Live systemd checks are also opt-in. On a Linux systemd host, run:

```sh
bash tests/fixtures/systemd.sh
```

This fixture script uses sudo to create two uniquely named units under
`/run/systemd/system`, starts/enables one, tests both, and removes them on exit.
It does not edit existing units. CI runs it on its Linux runner. The library's
service inspection methods are read-only.

See the [implementation plan](https://github.com/gilesw/testinfra-rs/blob/main/PLAN.md)
for the ordered slices and their validation. The first GitHub Actions run passed
live systemd validation as well as the Docker matrix.

## Releases

The [release guide](RELEASING.md) describes the reused tickerapp-rs automation:
`mise run bump` updates the version/changelog, `mise run tag` creates and pushes
an annotated release tag, `mise run release` validates a tagged dry run, and
`mise run release:publish` uploads that exact commit. Pushing later release tags
publishes through crates.io trusted publishing. The initial `0.1.0` upload is
manual because the crate must exist before configuring its trusted publisher.

Package metadata, a dated changelog, and an explicit archive file list are included.
The first release has been prepared; preparation and dry runs do not upload a crate.

## Existing alternatives

A search when starting this project found no direct Testinfra-style Rust port.
[testcontainers](https://docs.rs/testcontainers/latest/testcontainers/) and
[dockertest](https://docs.rs/dockertest/latest/dockertest/) provide Docker container
lifecycle and integration-test tooling. They serve a different need and can be
combined with this library to inspect containers they start. Python Testinfra
remains the established option if using Rust is not a requirement.
