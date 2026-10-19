use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use testinfra::{Error, Host, Result};

struct Files<'a> {
    host: &'a Host,
    root: String,
}
impl Drop for Files<'_> {
    fn drop(&mut self) {
        let _ = self.host.exec("rm", &["-rf", "--", &self.root]);
    }
}

fn exercise(host: &Host) -> Result<()> {
    let root = host
        .check_output("mktemp -d /tmp/testinfra-rs.XXXXXXXX")?
        .trim()
        .to_owned();
    assert!(root.starts_with("/tmp/testinfra-rs."));
    let _files = Files {
        host,
        root: root.clone(),
    };
    let path = format!("{root}/file '\"$()`;\nname");
    let link = format!("{root}/link");
    let dangling = format!("{root}/dangling");
    let setup = host.exec("sh", &["-c", "printf 'hello\\nroot:x:0\\n' > \"$1\" && chmod 640 \"$1\" && ln -s \"$1\" \"$2\" && ln -s /does-not-exist-testinfra \"$3\"", "setup", &path, &link, &dangling])?;
    assert!(setup.success(), "{setup:?}");

    let file = host.file(&path);
    assert!(file.exists()?);
    assert!(file.is_file()?);
    assert!(!file.is_directory()?);
    assert!(!file.is_symlink()?);
    assert_eq!(file.content()?, "hello\nroot:x:0\n");
    assert!(file.contains("(?m)^root:x:[0-9]$")?);
    assert!(!file.contains_literal("[0-9]")?);
    assert!(file.contains("[").is_err());
    let meta = file.metadata()?;
    assert_eq!(meta.mode, 0o640);
    assert_eq!(meta.size, 15);
    assert_eq!(meta.uid, 0);
    assert_eq!(file.uid()?, 0);
    assert_eq!(file.gid()?, 0);
    assert_eq!(file.size()?, 15);
    assert!(!file.is_executable()?);
    assert!(host.exec("chmod", &["750", &path])?.success());
    assert!(file.is_executable()?);
    assert!(host.exec("chmod", &["640", &path])?.success());
    assert_eq!(file.user()?, "root");
    assert_eq!(file.group()?, "root");
    assert!(host.file(&root).is_directory()?);
    assert!(host.file("").exists().is_err());
    assert!(host.file(&link).is_symlink()?);
    assert!(host.file(&link).is_file()?);
    assert_eq!(host.file(&link).link_target()?, path);
    assert!(file.link_target().is_err());
    assert_eq!(host.file(&link).metadata()?, meta);
    assert!(host.file(&dangling).exists()?);
    assert!(host.file(&dangling).is_symlink()?);
    assert!(!host.file(&dangling).is_file()?);
    let missing = format!("{root}/missing");
    assert!(!host.file(&missing).exists()?);
    assert!(host.file(&missing).content().is_err());
    assert!(host.file(&missing).metadata().is_err());

    let binary = format!("{root}/binary");
    assert!(
        host.exec(
            "sh",
            &["-c", "printf '\\377\\000' > \"$1\"", "setup", &binary]
        )?
        .success()
    );
    assert_eq!(host.file(&binary).content_bytes()?, [255, 0]);
    assert!(host.file(&binary).content().is_err());

    assert!(host.user("root").exists()?);
    assert_eq!(host.user("root").uid()?, 0);
    assert_eq!(host.user("root").gid()?, 0);
    assert_eq!(host.user("0").uid()?, 0);
    assert_eq!(host.user("root").home()?, "/root");
    assert!(host.user("root").shell()?.starts_with('/'));
    assert!(host.user("root").groups()?.contains(&"root".to_owned()));
    assert!(host.group("root").exists()?);
    assert_eq!(host.group("root").gid()?, 0);
    assert!(!host.user("testinfra-no-such-user-12345").exists()?);
    assert!(!host.group("testinfra-no-such-group-12345").exists()?);
    assert!(matches!(
        host.user("testinfra-no-such-user-12345").uid(),
        Err(Error::NotFound { kind: "user", .. })
    ));
    assert!(matches!(
        host.group("testinfra-no-such-group-12345").gid(),
        Err(Error::NotFound { kind: "group", .. })
    ));

    let info = host.system_info()?;
    let package = match info.distribution.as_str() {
        "alpine" => "busybox",
        "debian" | "ubuntu" => "base-files",
        "fedora" | "rocky" | "almalinux" => "filesystem",
        other => panic!("add a known installed package for {other}"),
    };
    assert!(!info.kernel.is_empty());
    assert!(!info.architecture.is_empty());
    assert_eq!(host.environment()?["LC_ALL"], "C");
    assert!(host.environment()?.contains_key("PATH"));
    assert!(host.sysctl("kernel.ostype").exists()?);
    assert_eq!(host.sysctl("kernel.ostype").value()?, "Linux");
    assert!(host.sysctl("kernel.pid_max").value_as::<u64>()? > 0);
    assert!(!host.sysctl("kernel.testinfra_missing_12345").exists()?);
    assert!(
        host.sysctl("kernel.testinfra_missing_12345")
            .value()
            .is_err()
    );
    assert!(host.sysctl("../etc/passwd").value().is_err());
    assert!(host.sysctl("kernel..pid_max").value().is_err());
    assert!(host.mount_point("/").is_mounted()?);
    assert_eq!(host.mount_point("/proc").info()?.filesystem, "proc");
    assert!(!host.mount_point(&root).is_mounted()?);
    assert!(matches!(
        host.mount_point(&root).info(),
        Err(Error::NotFound {
            kind: "mount point",
            ..
        })
    ));
    assert!(host.package(package).is_installed()?);
    assert!(!host.package(package).version()?.unwrap().is_empty());
    assert!(
        !host
            .package("testinfra-no-such-package-12345")
            .is_installed()?
    );
    assert_eq!(
        host.package("testinfra-no-such-package-12345").version()?,
        None
    );
    assert!(host.package("--help").is_installed().is_err());

    // Minimal images may omit pgrep. Missing tools must produce errors.
    if host.run("command -v pgrep >/dev/null")?.success() {
        assert!(!host.process("testinfra_nope").is_running()?);
        let sleep = host.exec(
            "sh",
            &["-c", "sleep 60 </dev/null >/dev/null 2>&1 & echo $!"],
        )?;
        let pid: u32 = sleep.stdout_text()?.trim().parse().unwrap();
        let pids = host.process("sleep").pids()?;
        let _ = host.exec("kill", &[&pid.to_string()]);
        assert!(pids.contains(&pid));
        assert!(host.process("[").pids().is_err());
    } else {
        assert!(host.process("sleep").is_running().is_err());
    }
    if !host.file("/run/systemd/system").is_directory()? {
        assert!(host.service("sshd").is_running().is_err());
        assert!(host.service("sshd").is_enabled().is_err());
    }
    Ok(())
}

struct Container(String);
impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

impl Container {
    fn start(image: &str, command: &[&str]) -> Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("testinfra-rs-{}-{unique}", std::process::id());
        let container = Container(name);
        let output = Command::new("docker")
            .args(["run", "--detach", "--name", &container.0, image])
            .args(command)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(container)
    }
}

#[test]
#[ignore = "requires Docker; creates and removes a Linux test container"]
fn docker_resources() -> Result<()> {
    let image = std::env::var("TESTINFRA_TEST_IMAGE").unwrap_or_else(|_| "alpine:3".into());
    let container = Container::start(&image, &["sh", "-c", "while :; do sleep 3600; done"])?;
    let host = Host::docker(&container.0)?;
    exercise(&host)?;
    let output = host.run("printf 'a\\000b'; printf problem >&2; exit 42")?;
    assert_eq!(output.stdout, b"a\0b");
    assert_eq!(output.stderr, "problem");
    assert_eq!(output.exit_status, 42);
    assert_eq!(host.run("exit 127")?.exit_status, 127);
    assert_eq!(host.check_output("printf ''")?, "");
    assert_eq!(host.run("head -c 65536 /dev/zero")?.stdout, vec![0; 65536]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    assert_eq!(
        runtime.block_on(async { host.check_output("printf async") })?,
        "async"
    );
    let unprivileged = Host::docker_as(&container.0, "65534:65534")?;
    assert_eq!(unprivileged.check_output("id -u")?.trim(), "65534");
    assert_eq!(
        Host::from_url(&format!("docker://root@{}", container.0))?
            .user("root")
            .uid()?,
        0
    );
    assert_eq!(host.run("exit 1")?.exit_status, 1);
    let output = Command::new("docker")
        .args(["stop", "--time", "1", &container.0])
        .output()?;
    assert!(output.status.success());
    assert!(matches!(
        host.file("/etc/passwd").exists(),
        Err(Error::Docker(_))
    ));
    assert!(Host::docker(&container.0).is_err());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn local_linux() -> Result<()> {
    let host = Host::local()?;
    assert!(host.file("/etc/passwd").is_file()?);
    assert!(host.file("/etc/passwd").contains("(?m)^root:")?);
    assert_eq!(host.user("root").uid()?, 0);
    assert!(!host.system_info()?.distribution.is_empty());
    assert_eq!(host.run("exit 37")?.exit_status, 37);
    Ok(())
}

#[test]
#[ignore = "requires Docker and the python:3.13-alpine fixture image"]
fn docker_sockets() -> Result<()> {
    let script = r#"
import os, socket, time
sockets = []
specs = []
for family, address in [(socket.AF_INET, '127.0.0.1'), (socket.AF_INET6, '::1')]:
    for kind, protocol in [(socket.SOCK_STREAM, 'tcp'), (socket.SOCK_DGRAM, 'udp')]:
        s = socket.socket(family, kind)
        try:
            s.bind((address, 0))
        except OSError:
            if family == socket.AF_INET6:
                s.close()
                continue
            raise
        if kind == socket.SOCK_STREAM:
            s.listen()
        sockets.append(s)
        host = '[' + address + ']' if family == socket.AF_INET6 else address
        specs.append(protocol + '://' + host + ':' + str(s.getsockname()[1]))
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind('/tmp/testinfra socket [*]')
s.listen()
sockets.append(s)
specs.append('unix:///tmp/testinfra socket [*]')
with open('/tmp/socket-fixture.tmp', 'w') as f:
    f.write('\n'.join(specs))
os.replace('/tmp/socket-fixture.tmp', '/tmp/socket-fixture.ready')
time.sleep(300)
"#;
    let container = Container::start("python:3.13-alpine", &["python3", "-c", script])?;
    let host = Host::docker(&container.0)?;
    let mut ready = false;
    for _ in 0..50 {
        if host.file("/tmp/socket-fixture.ready").exists()? {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ready, "socket fixture did not become ready");
    let specs = host.file("/tmp/socket-fixture.ready").content()?;
    assert!(specs.lines().count() >= 3);
    for spec in specs.lines() {
        assert!(host.socket(spec).is_listening()?, "{spec}");
        if spec.contains("127.0.0.1") {
            assert!(
                !host
                    .socket(&spec.replace("127.0.0.1", "127.0.0.2"))
                    .is_listening()?
            );
        }
    }
    assert!(!host.socket("unix:///tmp/testinfra socket").is_listening()?);
    Ok(())
}
