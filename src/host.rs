use std::{
    env,
    io::{self, ErrorKind},
    iter,
    process::Command,
    result, str, thread,
};
use tokio::runtime::Builder;

use bollard::{
    Docker,
    container::LogOutput,
    errors::Error as DockerError,
    exec::{CreateExecOptions, StartExecResults},
};
use futures_util::StreamExt;

/// Errors remain distinct from a successful check reporting an absent resource.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
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
    #[error("command {command:?} exited with {status}: {stderr}", status = .result.exit_status, stderr = .result.stderr.trim())]
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
    pub stderr: String,
}

impl CommandResult {
    pub fn success(&self) -> bool {
        self.exit_status == 0
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
}

impl Host {
    pub fn local() -> Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(Error::Unsupported(
                "local targets require Linux; use Docker on other hosts".into(),
            ));
        }
        Ok(Self {
            backend: Backend::Local,
        })
    }

    pub fn docker(container: &str) -> Result<Self> {
        Self::docker_inner(container, None)
    }

    /// Execute as a Docker username or UID, optionally followed by `:group`.
    pub fn docker_as(container: &str, user: &str) -> Result<Self> {
        if user.is_empty()
            || user.starts_with('-')
            || !user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        {
            return Err(Error::InvalidInput("invalid Docker user".into()));
        }
        Self::docker_inner(container, Some(user.to_owned()))
    }

    fn docker_inner(container: &str, user: Option<String>) -> Result<Self> {
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
                endpoint: docker_endpoint()?,
            },
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
    /// No timeout is imposed; callers must bound commands that may block.
    pub fn run(&self, script: &str) -> Result<CommandResult> {
        self.exec("sh", &["-c", script])
    }

    /// Execute a program directly with literal arguments, without a shell.
    /// A missing local executable returns `Error::Io`; Docker reports API errors.
    pub fn exec(&self, program: &str, args: &[&str]) -> Result<CommandResult> {
        if program.is_empty() || program.contains('\0') || args.iter().any(|arg| arg.contains('\0'))
        {
            return Err(Error::InvalidInput(
                "program must be nonempty and arguments cannot contain NUL".into(),
            ));
        }
        match &self.backend {
            Backend::Local => {
                let output = Command::new(program)
                    .args(args)
                    .env("LC_ALL", "C")
                    .output()?;
                Ok(CommandResult {
                    exit_status: output.status.code().ok_or_else(|| {
                        Error::Transport(format!("shell terminated by {}", output.status))
                    })?,
                    stdout: output.stdout,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                })
            }
            Backend::Docker {
                container,
                user,
                endpoint,
            } => {
                // Keep the synchronous API usable from ordinary tests and Tokio tests.
                // A scoped thread prevents nested-runtime panics in async callers.
                thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            Builder::new_current_thread()
                                .enable_all()
                                .build()?
                                .block_on(docker_run(
                                    container,
                                    user.as_deref(),
                                    endpoint.as_deref(),
                                    program,
                                    args,
                                ))
                        })
                        .join()
                        .map_err(|_| Error::Transport("Docker execution worker panicked".into()))?
                })
            }
        }
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

// Bollard handles Docker's wire protocol. Ask the CLI to resolve its selected
// context when available, rather than implementing Docker config/context parsing.
fn docker_endpoint() -> Result<Option<String>> {
    let has_context = env::var_os("DOCKER_CONTEXT").is_some_and(|s| !s.is_empty());
    if !has_context && env::var_os("DOCKER_HOST").is_some_and(|s| !s.is_empty()) {
        return Ok(None);
    }
    match Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let endpoint =
                String::from_utf8(output.stdout).map_err(|e| Error::Parse(e.to_string()))?;
            Ok(Some(endpoint.trim().to_owned()))
        }
        Err(e) if e.kind() == ErrorKind::NotFound && !has_context => Ok(None),
        Err(e) => Err(e.into()),
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
    drop(input);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(chunk) = output.next().await {
        match chunk? {
            LogOutput::StdOut { message } | LogOutput::Console { message } => {
                stdout.extend_from_slice(&message)
            }
            LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
            LogOutput::StdIn { .. } => {}
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
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
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
        }
    }

    #[test]
    fn status_stdout_stderr_and_binary_are_preserved() -> Result<()> {
        let r = shell().run("printf 'a\\000b'; printf problem >&2; exit 42")?;
        assert_eq!(r.stdout, b"a\0b");
        assert_eq!(r.stderr, "problem");
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
