use std::{
    borrow::Cow,
    env,
    future::Future,
    io::{self, ErrorKind},
    iter,
    process::{Output, Stdio},
    result, str, thread,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    process::Command,
    runtime::Builder,
    time::timeout,
};

use bollard::{
    Docker,
    container::LogOutput,
    errors::Error as DockerError,
    exec::{CreateExecOptions, StartExecResults},
};
use futures_util::{
    StreamExt,
    future::{Either, select},
};

use crate::capture::{self, OutputLimits, OutputStream};

/// Errors remain distinct from a successful check reporting an absent resource.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The wait expired. A remote command or local descendants may still run.
    #[error("system check timed out after {timeout:?}")]
    Timeout { timeout: Duration },
    /// Capture stopped without returning partial output. Remote work may continue.
    #[error("{stream} exceeded the capture limit of {limit} bytes")]
    OutputLimitExceeded { stream: OutputStream, limit: usize },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("{kind} {name:?} does not exist")]
    NotFound { kind: &'static str, name: String },
    #[error("execution failed: {0}")]
    Transport(String),
    #[error(transparent)]
    Docker(#[from] DockerError),
    #[error("command {command:?} exited with {status}: {stderr}", status = .result.exit_status, stderr = .result.stderr_text_lossy().trim())]
    CommandFailed {
        command: String,
        result: CommandResult,
    },
    #[error("invalid command output: {0}")]
    Parse(String),
}

pub type Result<T> = result::Result<T, Error>;

/// Raw bytes preserve binary file contents and non-UTF-8 command output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandResult {
    pub fn success(&self) -> bool {
        self.exit_status == 0
    }
    /// Decode stderr as UTF-8 without replacing invalid bytes.
    pub fn stderr_text(&self) -> Result<&str> {
        str::from_utf8(&self.stderr).map_err(|e| Error::Parse(e.to_string()))
    }
    /// Display stderr with replacement characters for invalid UTF-8.
    pub fn stderr_text_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
    pub fn stdout_text(&self) -> Result<&str> {
        str::from_utf8(&self.stdout).map_err(|e| Error::Parse(e.to_string()))
    }
}

#[derive(Debug, Clone)]
enum Backend {
    Local,
    Docker {
        container: String,
        user: Option<String>,
        endpoint: Option<String>,
    },
}

/// A local Linux machine or an existing, running Linux Docker container.
#[derive(Debug, Clone)]
pub struct Host {
    backend: Backend,
    timeout: Duration,
    output_limits: OutputLimits,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

impl Host {
    pub fn local() -> Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(Error::Unsupported(
                "local targets require Linux; use Docker on other hosts".into(),
            ));
        }
        Ok(Self {
            backend: Backend::Local,
            timeout: DEFAULT_TIMEOUT,
            output_limits: OutputLimits::default(),
        })
    }

    pub fn docker(container: &str) -> Result<Self> {
        Self::docker_with_timeout(container, DEFAULT_TIMEOUT)
    }

    /// Construct a Docker host with a timeout for discovery and each command.
    pub fn docker_with_timeout(container: &str, timeout: Duration) -> Result<Self> {
        Self::docker_inner(container, None, timeout)
    }

    /// Set the timeout for each subsequent command, including resource reads.
    /// A resource method that executes several commands gets this budget per command.
    /// Clone the host first to give a single check a different budget.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self> {
        validate_timeout(timeout)?;
        self.timeout = timeout;
        Ok(self)
    }

    /// Set the per-command stdout and stderr capture limits in bytes.
    /// Defaults to 4 MiB per stream. Zero permits only empty output.
    /// Resource reads inherit these limits; clone first for a single check.
    pub fn with_output_limits(mut self, stdout: usize, stderr: usize) -> Self {
        self.output_limits = OutputLimits { stdout, stderr };
        self
    }

    /// Execute as a Docker username or UID, optionally followed by `:group`.
    pub fn docker_as(container: &str, user: &str) -> Result<Self> {
        Self::docker_as_with_timeout(container, user, DEFAULT_TIMEOUT)
    }

    /// Like `docker_as`, with a timeout for discovery and each command.
    pub fn docker_as_with_timeout(container: &str, user: &str, timeout: Duration) -> Result<Self> {
        if user.is_empty()
            || user.starts_with('-')
            || !user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        {
            return Err(Error::InvalidInput("invalid Docker user".into()));
        }
        Self::docker_inner(container, Some(user.to_owned()), timeout)
    }

    fn docker_inner(container: &str, user: Option<String>, timeout: Duration) -> Result<Self> {
        validate_timeout(timeout)?;
        if container.is_empty()
            || !container.as_bytes()[0].is_ascii_alphanumeric()
            || !container
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err(Error::InvalidInput(
                "expected a Docker container name or ID".into(),
            ));
        }
        let host = Self {
            backend: Backend::Docker {
                container: container.into(),
                user,
                endpoint: run_sync(docker_endpoint(timeout))?,
            },
            timeout,
            output_limits: OutputLimits::default(),
        };
        if host.check_output("uname -s")?.trim() != "Linux" {
            return Err(Error::Unsupported("Docker target must run Linux".into()));
        }
        Ok(host)
    }

    /// Accept `local://` or `docker://[user@]container`.
    pub fn from_url(url: &str) -> Result<Self> {
        if url == "local://" {
            return Self::local();
        }
        if let Some(target) = url.strip_prefix("docker://") {
            return match target.split_once('@') {
                Some((user, container)) => Self::docker_as(container, user),
                None => Self::docker(target),
            };
        }
        Err(Error::InvalidInput(format!("unsupported host URL: {url}")))
    }

    /// Read `TESTINFRA_HOST`, defaulting to `local://` when unset.
    pub fn from_env() -> Result<Self> {
        match env::var("TESTINFRA_HOST") {
            Ok(url) => Self::from_url(&url),
            Err(env::VarError::NotPresent) => Self::local(),
            Err(e) => Err(Error::InvalidInput(e.to_string())),
        }
    }

    /// Run an explicit shell script. Nonzero target status is returned as data.
    /// Uses the host timeout (30 seconds by default).
    pub fn run(&self, script: &str) -> Result<CommandResult> {
        self.exec("sh", &["-c", script])
    }

    /// Run explicit shell code with binary stdin, then close stdin.
    /// The host timeout covers writing input and collecting output together.
    pub fn run_with_input(&self, script: &str, input: &[u8]) -> Result<CommandResult> {
        self.exec_with_input("sh", &["-c", script], input)
    }

    /// Execute a program directly with literal arguments, without a shell.
    /// A missing local executable returns `Error::Io`; Docker reports API errors.
    pub fn exec(&self, program: &str, args: &[&str]) -> Result<CommandResult> {
        self.exec_with_input(program, args, &[])
    }

    /// Execute literal arguments with binary stdin, then close stdin.
    ///
    /// ```no_run
    /// # use testinfra::{Host, Result};
    /// # fn example(host: &Host) -> Result<()> {
    /// let result = host.exec_with_input("cat", &[], b"binary\0input")?;
    /// assert_eq!(result.stdout, b"binary\0input");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Input and output are transferred concurrently under the host timeout.
    /// An early exit can leave some input unread; its exit status remains data.
    pub fn exec_with_input(
        &self,
        program: &str,
        args: &[&str],
        input: &[u8],
    ) -> Result<CommandResult> {
        if program.is_empty() || program.contains('\0') || args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(Error::InvalidInput(
                "program must be nonempty and arguments cannot contain NUL".into(),
            ));
        }
        run_sync(async {
            match &self.backend {
                Backend::Local => {
                    let output = local_output(
                        Command::new(program).args(args).env("LC_ALL", "C"),
                        self.timeout,
                        input,
                        self.output_limits,
                    )
                    .await?;
                    Ok(CommandResult {
                        exit_status: output.status.code().ok_or_else(|| {
                            Error::Transport(format!("command terminated by {}", output.status))
                        })?,
                        stdout: output.stdout,
                        stderr: output.stderr,
                    })
                }
                Backend::Docker {
                    container,
                    user,
                    endpoint,
                } => timeout(
                    self.timeout,
                    docker_run(
                        container,
                        user.as_deref(),
                        endpoint.as_deref(),
                        program,
                        args,
                        input,
                        self.output_limits,
                    ),
                )
                .await
                .map_err(|_| Error::Timeout {
                    timeout: self.timeout,
                })?,
            }
        })
    }

    /// Run shell code and require one of the listed exit statuses.
    pub fn run_expect(&self, expected: &[i32], script: &str) -> Result<CommandResult> {
        if expected.is_empty() {
            return Err(Error::InvalidInput(
                "expected exit statuses cannot be empty".into(),
            ));
        }
        let result = self.run(script)?;
        if expected.contains(&result.exit_status) {
            Ok(result)
        } else {
            Err(Error::CommandFailed {
                command: script.into(),
                result,
            })
        }
    }

    pub fn check_output(&self, script: &str) -> Result<String> {
        let result = self.run_expect(&[0], script)?;
        Ok(result.stdout_text()?.to_owned())
    }

    pub(crate) fn predicate(&self, script: &str) -> Result<bool> {
        Ok(self.run_expect(&[0, 1], script)?.success())
    }
}

fn validate_timeout(limit: Duration) -> Result<()> {
    if limit.is_zero() || Instant::now().checked_add(limit).is_none() {
        return Err(Error::InvalidInput(
            "timeout must be positive and representable".into(),
        ));
    }
    Ok(())
}

// A scoped worker keeps synchronous calls usable inside an existing Tokio runtime.
fn run_sync<T: Send>(future: impl Future<Output = Result<T>> + Send) -> Result<T> {
    thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = Builder::new_current_thread().enable_all().build()?;
                let result = runtime.block_on(future);
                // Resolver work may use blocking threads; shutdown must not extend a deadline.
                runtime.shutdown_background();
                result
            })
            .join()
            .map_err(|_| Error::Transport("execution worker panicked".into()))?
    })
}

async fn local_output(
    command: &mut Command,
    limit: Duration,
    input: &[u8],
    limits: OutputLimits,
) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Transport("missing stdin pipe".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Transport("missing stdout pipe".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Transport("missing stderr pipe".into()))?;
    let result = timeout(limit, async {
        let (status, out, err, _) = futures_util::try_join!(
            async { child.wait().await.map_err(Error::from) },
            capture::read(&mut stdout, limits.stdout, OutputStream::Stdout),
            capture::read(&mut stderr, limits.stderr, OutputStream::Stderr),
            async { write_input(stdin, input).await.map_err(Error::from) }
        )?;
        Ok::<_, Error>(Output {
            status,
            stdout: out,
            stderr: err,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        other => {
            // Close pipes without waiting for descendants; bound direct-child cleanup too.
            drop(stdout);
            drop(stderr);
            let _ = timeout(Duration::from_secs(1), child.kill()).await;
            match other {
                Ok(Err(error)) => Err(error),
                Err(_) => Err(Error::Timeout { timeout: limit }),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}

async fn write_input(mut writer: impl AsyncWrite + Unpin, input: &[u8]) -> io::Result<()> {
    let result = async {
        writer.write_all(input).await?;
        writer.shutdown().await
    }
    .await;
    match result {
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

// Bollard handles Docker's wire protocol. Ask the CLI to resolve its selected
// context when available, rather than implementing Docker config/context parsing.
async fn docker_endpoint(limit: Duration) -> Result<Option<String>> {
    let has_context = env::var_os("DOCKER_CONTEXT").is_some_and(|s| !s.is_empty());
    if !has_context && env::var_os("DOCKER_HOST").is_some_and(|s| !s.is_empty()) {
        return Ok(None);
    }
    match local_output(
        Command::new("docker").args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ]),
        limit,
        &[],
        OutputLimits::default(),
    )
    .await
    {
        Ok(output) if output.status.success() => {
            let endpoint =
                String::from_utf8(output.stdout).map_err(|e| Error::Parse(e.to_string()))?;
            Ok(Some(endpoint.trim().to_owned()))
        }
        Err(Error::Io(e)) if e.kind() == ErrorKind::NotFound && !has_context => Ok(None),
        Err(e) => Err(e),
        Ok(output) => Err(Error::Transport(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )),
    }
}

async fn docker_run(
    container: &str,
    user: Option<&str>,
    endpoint: Option<&str>,
    program: &str,
    args: &[&str],
    payload: &[u8],
    limits: OutputLimits,
) -> Result<CommandResult> {
    let docker = match endpoint {
        Some(endpoint) => Docker::connect_with_host(endpoint)?,
        None => Docker::connect_with_defaults()?,
    }
    .negotiate_version()
    .await?;
    let command: Vec<&str> = iter::once(program).chain(args.iter().copied()).collect();
    let exec = docker
        .create_exec(
            container,
            CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                env: Some(vec!["LC_ALL=C"]),
                cmd: Some(command),
                user,
                ..Default::default()
            },
        )
        .await?;
    let StartExecResults::Attached { mut output, input } =
        docker.start_exec(&exec.id, None).await?
    else {
        return Err(Error::Transport(
            "Docker unexpectedly detached execution".into(),
        ));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let read = async {
        while let Some(chunk) = output.next().await {
            match chunk? {
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    capture::append(&mut stdout, &message, limits.stdout, OutputStream::Stdout)?
                }
                LogOutput::StdErr { message } => {
                    capture::append(&mut stderr, &message, limits.stderr, OutputStream::Stderr)?
                }
                LogOutput::StdIn { .. } => {}
            }
        }
        Ok::<_, Error>(())
    };
    let write = async { write_input(input, payload).await.map_err(Error::from) };
    match select(Box::pin(read), Box::pin(write)).await {
        Either::Left((result, pending_write)) => {
            // Output EOF can arrive while Docker still accepts input for an exited command.
            drop(pending_write);
            result?;
        }
        Either::Right((result, pending_read)) => {
            result?;
            pending_read.await?;
        }
    }
    let state = docker.inspect_exec(&exec.id).await?;
    if state.running == Some(true) {
        return Err(Error::Transport(
            "Docker output stream closed before command exited".into(),
        ));
    }
    let exit_status = state
        .exit_code
        .and_then(|code| i32::try_from(code).ok())
        .ok_or_else(|| Error::Transport("Docker did not report a command exit status".into()))?;
    Ok(CommandResult {
        exit_status,
        stdout,
        stderr,
    })
}

pub(crate) fn quote(value: &str) -> Result<String> {
    shlex::try_quote(value)
        .map(|s| s.into_owned())
        .map_err(|e| Error::InvalidInput(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> Host {
        Host {
            backend: Backend::Local,
            timeout: DEFAULT_TIMEOUT,
            output_limits: OutputLimits::default(),
        }
    }

    #[test]
    fn timeout_is_an_error_and_the_host_remains_usable() -> Result<()> {
        let host = shell().with_timeout(Duration::from_secs(1))?;
        let started = Instant::now();
        assert!(matches!(
            host.exec("sleep", &["30"]),
            Err(Error::Timeout { .. })
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(host.check_output("printf recovered")?, "recovered");
        assert!(matches!(
            host.run_expect(&[0], "exec sleep 30"),
            Err(Error::Timeout { .. })
        ));
        Ok(())
    }

    #[test]
    fn inherited_pipes_do_not_extend_the_deadline() -> Result<()> {
        let host = shell().with_timeout(Duration::from_secs(1))?;
        let started = Instant::now();
        assert!(matches!(
            host.run("sleep 5 & exit 0"),
            Err(Error::Timeout { .. })
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        Ok(())
    }

    #[test]
    fn timeouts_work_inside_a_current_thread_runtime() -> Result<()> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(async {
            let host = shell().with_timeout(Duration::from_secs(1))?;
            assert!(matches!(
                host.run("exec sleep 30"),
                Err(Error::Timeout { .. })
            ));
            assert!(host.run("true")?.success());
            Ok(())
        })
    }

    #[test]
    fn invalid_timeouts_fail_before_discovery() {
        assert!(matches!(
            shell().with_timeout(Duration::ZERO),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            Host::docker_with_timeout("example", Duration::MAX),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn status_stdout_stderr_and_binary_are_preserved() -> Result<()> {
        let r = shell().run("printf 'a\\000b'; printf problem >&2; exit 42")?;
        assert_eq!(r.stdout, b"a\0b");
        assert_eq!(r.stderr, b"problem");
        assert_eq!(r.exit_status, 42);
        assert!(!r.success());
        assert_eq!(shell().run("exit 127")?.exit_status, 127);
        Ok(())
    }

    #[test]
    fn arguments_are_literal() -> Result<()> {
        let value = "'; $(printf injected) `printf injected`\n--flag";
        let r = shell().exec("printf", &["%s", value])?;
        assert_eq!(r.stdout_text()?, value);
        assert!(shell().exec("printf", &["\0"]).is_err());
        assert!(shell().exec("", &[]).is_err());
        assert!(shell().run("printf \0").is_err());
        assert!(matches!(
            shell().exec("testinfra-no-such-command-12345", &[]),
            Err(Error::Io(_))
        ));
        Ok(())
    }

    #[test]
    fn invalid_targets_are_errors() {
        assert!(Host::docker("--privileged").is_err());
        assert!(Host::from_url("ssh://example").is_err());
    }

    #[test]
    fn check_failures_are_not_absence() {
        assert!(shell().run_expect(&[], "true").is_err());
        assert_eq!(shell().run_expect(&[7], "exit 7").unwrap().exit_status, 7);
        assert!(matches!(
            shell().predicate("exit 2"),
            Err(Error::CommandFailed { .. })
        ));
        assert!(matches!(
            shell().check_output("exit 1"),
            Err(Error::CommandFailed { .. })
        ));
    }
}
