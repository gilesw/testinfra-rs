use std::{
    env,
    process::{self, Command},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use testinfra::{Error, Host, Result};
use tokio::runtime::Builder;

#[path = "support/output_capture.rs"]
mod output_capture;

fn exercise(host: &Host) -> Result<()> {
    binary_input_and_eof(host)?;
    simultaneous_streams(host)?;
    early_exit(host)?;
    blocked_input(host)?;
    output_capture::exercise(host)?;
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let output = runtime.block_on(async { host.exec_with_input("cat", &[], b"runtime") })?;
    assert_eq!(output.stdout, b"runtime");
    Ok(())
}

fn binary_input_and_eof(host: &Host) -> Result<()> {
    let payload = b"\0\xff\n'\"$()`; input";
    let output = host.exec_with_input("cat", &[], payload)?;
    assert_eq!(output.stdout, payload);
    assert!(output.stderr.is_empty());
    assert!(output.success());
    assert!(host.exec_with_input("cat", &[], &[])?.stdout.is_empty());
    assert!(host.exec("cat", &[])?.stdout.is_empty());
    assert!(host.run("cat")?.stdout.is_empty());
    let output = host.run_with_input("cat; printf error >&2; exit 37", payload)?;
    assert_eq!(output.stdout, payload);
    assert_eq!(output.stderr, b"error");
    assert_eq!(output.exit_status, 37);
    let literal = "'$(printf injected)`printf injected`\n--flag";
    let output = host.exec_with_input("printf", &["%s", literal], payload)?;
    assert_eq!(output.stdout_text()?, literal);
    assert!(matches!(
        host.exec_with_input("", &[], payload),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        host.exec_with_input("cat", &["\0"], payload),
        Err(Error::InvalidInput(_))
    ));
    Ok(())
}

fn simultaneous_streams(host: &Host) -> Result<()> {
    let size = 1024 * 1024;
    let payload: Vec<u8> = (0..=255).cycle().take(size).collect();
    // Output larger than a pipe before reading input requires concurrent transfer.
    let output = host.run_with_input(
        "head -c 1048576 /dev/zero; head -c 1048576 /dev/zero >&2; cat; exit 37",
        &payload,
    )?;
    assert_eq!(output.exit_status, 37);
    assert_eq!(output.stdout.len(), 2 * size);
    assert!(output.stdout[..size].iter().all(|byte| *byte == 0));
    assert_eq!(output.stdout[size..], payload);
    assert_eq!(output.stderr.as_slice(), vec![0; size]);
    Ok(())
}

fn early_exit(host: &Host) -> Result<()> {
    let payload = vec![b'x'; 8 * 1024 * 1024];
    for script in [
        "printf done; printf error >&2; exit 37",
        "head -c 1 >/dev/null; printf done; printf error >&2; exit 37",
        "exec 0<&-; printf done; printf error >&2; exit 37",
    ] {
        let output = host.run_with_input(script, &payload)?;
        assert_eq!(output.exit_status, 37);
        assert_eq!(output.stdout, b"done");
        assert_eq!(output.stderr, b"error");
    }
    assert!(host.exec_with_input("true", &[], &payload)?.success());
    Ok(())
}

fn blocked_input(host: &Host) -> Result<()> {
    let payload = vec![b'x'; 16 * 1024 * 1024];
    let host = host.clone().with_timeout(Duration::from_secs(1))?;
    let started = Instant::now();
    assert!(matches!(
        host.exec_with_input("sleep", &["30"], &payload),
        Err(Error::Timeout { .. })
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(matches!(
        host.run_with_input("cat >/dev/null; exec sleep 30", b"consumed"),
        Err(Error::Timeout { .. })
    ));
    assert_eq!(
        host.exec_with_input("cat", &[], b"recovered")?.stdout,
        b"recovered"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn local_input() -> Result<()> {
    exercise(&Host::local()?)
}

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

#[test]
#[ignore = "requires Docker; creates and removes a Linux test container"]
fn docker_input() -> Result<()> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let container = Container(format!("testinfra-input-{}-{unique}", process::id()));
    let image = env::var("TESTINFRA_TEST_IMAGE").unwrap_or_else(|_| "alpine:3".into());
    let output = Command::new("docker")
        .args(["run", "-d", "--name", &container.0, &image, "sleep", "300"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    exercise(&Host::docker(&container.0)?)
}
