#![cfg(unix)]

use std::{
    env, fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};
use testinfra::{Error, Host, Result};

#[test]
fn discovery_and_probe_share_one_budget() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("tcp://{}", listener.local_addr()?);
    let docker = directory.path().join("docker");
    fs::write(
        &docker,
        format!("#!/bin/sh\n/bin/sleep 2\nprintf '%s\\n' '{endpoint}'\n"),
    )?;
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o700))?;
    // The listening socket completes TCP setup but never answers the API probe.
    let output = Command::new(env::current_exe()?)
        .args(["--exact", "constructor_child", "--ignored", "--nocapture"])
        .env("PATH", directory.path())
        .env("DOCKER_CONTEXT", "testinfra-constructor-fixture")
        .env_remove("DOCKER_HOST")
        .env("TESTINFRA_CONSTRUCTOR_CHILD", "1")
        .output()?;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
#[ignore = "subprocess fixture for discovery_and_probe_share_one_budget"]
fn constructor_child() {
    assert_eq!(env::var("TESTINFRA_CONSTRUCTOR_CHILD").as_deref(), Ok("1"));
    let started = Instant::now();
    assert!(matches!(
        Host::docker_with_timeout("fixture", Duration::from_secs(3)),
        Err(Error::Timeout { .. })
    ));
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_millis(4500));
}
