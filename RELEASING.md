# Releasing testinfra

Release automation and its isolated tests are reused from
[tickerapp-rs](https://github.com/gilesw/tickerapp-rs). Rust and tooling versions
are pinned in `mise.toml`; `Cargo.toml` declares the same Rust 1.98.1 minimum.
Lint scheduling lives in `hk.pkl`, using the shared local and CI gate. Mise
installs and exposes all development tools; hk runs the lint commands.

## Validate a candidate

From a Git checkout with the mise tools installed:

```sh
mise install
mise run check
mise run test:docker
TESTINFRA_TEST_IMAGE=debian:bookworm-slim mise run test:docker
TESTINFRA_TEST_IMAGE=fedora:44 mise run test:docker
mise run test:sockets
cargo package --list
mise run package:check
```

`package:check` runs Cargo's dry run, builds the extracted archive, and never
uploads. Docker tests create their own fixtures. On a Linux systemd host,
`mise run test:systemd` creates and cleans up its own runtime units using sudo.
The reusable CI test workflow includes Linux, live systemd, socket, packaging,
and all three Docker images. The lint workflow runs the full hk profile:
scripts, actions, dependencies, formatting, Clippy, and rustdoc. Both workflows
must pass before publishing. Release-script tests run in the test workflow.

The crate archive includes source, the executable example, integration tests and
their systemd fixture, README, release guide, changelog, and license. CI/release
automation remains in Git.
Packaging and dry runs do not reserve a crate name or prove upload credentials.

## First release

The prepared candidate is `testinfra` version `0.1.0`. Before uploading:

1. Confirm the exact name/version remains available on crates.io.
2. Commit and push the candidate to `main` and wait for both workflows to pass.
3. Review `CHANGELOG.md` and the output of `cargo package --list`.
4. Authenticate with an inherited `CARGO_REGISTRY_TOKEN`, `cargo login`, or an
   existing Cargo credential provider. Do not put tokens in files in this repo
   or pass them as command-line arguments.
5. After approval to publish, run `mise run tag` to create and push the annotated
   `v0.1.0` tag, then wait for its lint/test jobs. The workflow deliberately skips
   automatic uploading for this bootstrap tag. Run `mise run release` for the
   complete tagged dry run and `mise run release:publish` to upload locally.
   Verify the result on crates.io before retrying an upload that reports a timeout.
6. Register a trusted publisher in the crate's crates.io settings: owner `gilesw`,
   repository `testinfra-rs`, workflow `release.yml`, environment `crates-io`.
7. Later version tags publish through OIDC. The bootstrap `v0.1.0` upload remains
   manual so its tag can be pushed and tested before the crate/publisher exists.

An upload is permanent for that version. This preparation does not authorize or
perform an upload or push a release tag. Trusted publishing currently needs an
existing crate; the first upload uses the normal Cargo credential flow.
[Cargo publishing guide](https://doc.rust-lang.org/cargo/reference/publishing.html)
and [crates.io trusted publishing](https://crates.io/docs/trusted-publishing).

## Later releases

```sh
mise run bump patch        # or minor / major; edit version, lockfile, changelog
mise run check
git diff
```

The bump script uses `git-cliff`, requires the current version's tag, and creates
no commits or refs. Review and commit its output, merge/push to `main`, and wait
for CI. Then `mise run tag` checks the clean tree, version/changelog, and exact
`origin/main` tip, creates an annotated `vVERSION` tag, and pushes it. Pushing a
new release tag starts publication using a short-lived OIDC token.

The publish job is bound to the `crates-io` environment, requests `id-token: write`,
waits for lint and the complete test workflow, and serializes uploads. It calls
`mise run release:publish`, which verifies local/remote tag identity, repeats the
common gate and package dry run, then invokes Cargo. It never changes files or
Git refs. For an already pushed tag, `mise run release` runs the same checks
without uploading. Keep published tags unchanged; fixes need another version.

## Maintaining automation

`mise run lint:scripts` selects ShellCheck through hk. `mise run test:scripts`
runs the copied release-script tests, and the normal test/check gates include
them. They use temporary Git repositories and stubbed tools; their publish cases
never contact a registry or upload. hk's fast checks omit test suites and compile
checks; `mise run lint` enables its `slow` profile for Clippy/rustdoc. The
first-release workflow condition is checked by actionlint and should be reviewed
if changing the bootstrap version.
