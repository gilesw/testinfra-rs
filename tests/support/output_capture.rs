use std::time::{Duration, Instant};

use testinfra::{Error, Host, OutputStream, Result};

pub fn exercise(host: &Host) -> Result<()> {
    binary_stderr(host)?;
    boundaries(host)?;
    blocked_input_overflow(host)?;
    resource_limits(host)?;
    Ok(())
}

fn binary_stderr(host: &Host) -> Result<()> {
    let bytes = b"\xff\0\x80\n";
    let result = host.run_with_input("cat >&2; exit 37", bytes)?;
    assert_eq!(result.stderr, bytes);
    assert_eq!(result.exit_status, 37);
    assert!(matches!(result.stderr_text(), Err(Error::Parse(_))));
    assert_eq!(result.stderr_text_lossy(), String::from_utf8_lossy(bytes));
    let error = host
        .run_expect(&[0], "printf '\\377\\000' >&2; exit 37")
        .unwrap_err();
    assert!(error.to_string().contains("exited with 37"));
    match error {
        Error::CommandFailed { result, .. } => assert_eq!(result.stderr, b"\xff\0"),
        other => panic!("expected command failure, got {other:?}"),
    }
    assert_eq!(host.run("printf text >&2")?.stderr_text()?, "text");
    Ok(())
}

fn assert_overflow(error: Error, expected_stream: OutputStream, expected_limit: usize) {
    match error {
        Error::OutputLimitExceeded { stream, limit } => {
            assert_eq!(stream, expected_stream);
            assert_eq!(limit, expected_limit);
        }
        other => panic!("expected capture overflow, got {other:?}"),
    }
}

fn boundaries(host: &Host) -> Result<()> {
    let bounded = host.clone().with_output_limits(3, 2);
    let result = bounded.run("printf abc; printf de >&2; exit 37")?;
    assert_eq!(result.stdout, b"abc");
    assert_eq!(result.stderr, b"de");
    assert_eq!(result.exit_status, 37);
    assert_overflow(
        bounded.run("printf abcd").unwrap_err(),
        OutputStream::Stdout,
        3,
    );
    assert_overflow(
        bounded.run("printf def >&2").unwrap_err(),
        OutputStream::Stderr,
        2,
    );
    let silent = host.clone().with_output_limits(0, 0);
    assert!(silent.run("true")?.success());
    assert_overflow(silent.run("printf a").unwrap_err(), OutputStream::Stdout, 0);
    assert_overflow(
        silent.run("printf a >&2").unwrap_err(),
        OutputStream::Stderr,
        0,
    );
    let byte = host.clone().with_output_limits(1, 1);
    assert_overflow(
        byte.run("printf '\\303\\251' >&2").unwrap_err(),
        OutputStream::Stderr,
        1,
    );
    let bounded = host.clone().with_output_limits(65536, 65536);
    let result = bounded.run("head -c 65536 /dev/zero; head -c 65536 /dev/zero >&2")?;
    assert_eq!(result.stdout.len(), 65536);
    assert_eq!(result.stderr.len(), 65536);
    assert_overflow(
        bounded.run("head -c 65537 /dev/zero").unwrap_err(),
        OutputStream::Stdout,
        65536,
    );
    assert_overflow(
        bounded.run("head -c 65537 /dev/zero >&2").unwrap_err(),
        OutputStream::Stderr,
        65536,
    );
    let maximum = host.clone().with_output_limits(usize::MAX, usize::MAX);
    assert_eq!(maximum.run("printf small")?.stdout, b"small");
    assert_eq!(bounded.run("printf recovered")?.stdout, b"recovered");
    Ok(())
}

fn blocked_input_overflow(host: &Host) -> Result<()> {
    let bounded = host.clone().with_output_limits(1024, 1024);
    let input = vec![b'x'; 8 * 1024 * 1024];
    for (script, stream) in [
        (
            "head -c 2048 /dev/zero; exec sleep 30",
            OutputStream::Stdout,
        ),
        (
            "head -c 2048 /dev/zero >&2; exec sleep 30",
            OutputStream::Stderr,
        ),
    ] {
        let started = Instant::now();
        assert_overflow(
            bounded.run_with_input(script, &input).unwrap_err(),
            stream,
            1024,
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    let error = bounded
        .clone()
        .with_output_limits(1, 0)
        .run_with_input("cat", b"private-payload-marker")
        .unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("private-payload-marker"));
    assert_eq!(
        bounded
            .exec_with_input("cat", &[], b"after overflow")?
            .stdout,
        b"after overflow"
    );
    Ok(())
}

struct Directory<'a> {
    host: &'a Host,
    path: String,
}

impl Drop for Directory<'_> {
    fn drop(&mut self) {
        let _ = self.host.exec("rm", &["-rf", "--", &self.path]);
    }
}

fn resource_limits(host: &Host) -> Result<()> {
    let directory = Directory {
        host,
        path: host
            .check_output("mktemp -d /tmp/testinfra-capture.XXXXXXXX")?
            .trim()
            .to_owned(),
    };
    let path = format!("{}/large", directory.path);
    let output = host.exec(
        "sh",
        &["-c", "head -c 4194305 /dev/zero > \"$1\"", "fixture", &path],
    )?;
    assert!(output.success());
    let limit = 4 * 1024 * 1024;
    assert_overflow(
        host.file(&path).content_bytes().unwrap_err(),
        OutputStream::Stdout,
        limit,
    );
    assert_overflow(
        host.run("head -c 4194305 /dev/zero >&2").unwrap_err(),
        OutputStream::Stderr,
        limit,
    );
    let larger = host.clone().with_output_limits(limit + 1, 0);
    assert_eq!(larger.file(&path).content_bytes()?.len(), limit + 1);
    assert!(host.file("/etc/passwd").is_file()?);
    Ok(())
}
