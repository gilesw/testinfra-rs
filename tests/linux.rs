#![cfg(target_os = "linux")]

use std::{
    net::{TcpListener, UdpSocket},
    os::unix::net::UnixListener,
};
use testinfra::{Host, Result};

#[test]
fn listeners_follow_actual_kernel_state() -> Result<()> {
    let host = Host::local()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    assert!(host.socket(&format!("tcp://{address}")).is_listening()?);
    assert!(
        host.socket(&format!("tcp://{}", address.port()))
            .is_listening()?
    );
    assert!(
        !host
            .socket(&format!("tcp://127.0.0.2:{}", address.port()))
            .is_listening()?
    );
    drop(listener);
    assert!(!host.socket(&format!("tcp://{address}")).is_listening()?);

    let udp = UdpSocket::bind("127.0.0.1:0")?;
    let address = udp.local_addr()?;
    assert!(host.socket(&format!("udp://{address}")).is_listening()?);
    udp.connect("127.0.0.1:9")?;
    assert!(!host.socket(&format!("udp://{address}")).is_listening()?);

    if host.file("/proc/self/net/tcp6").exists()? {
        let listener = TcpListener::bind("[::1]:0")?;
        assert!(
            host.socket(&format!("tcp://{}", listener.local_addr()?))
                .is_listening()?
        );
        let udp = UdpSocket::bind("[::1]:0")?;
        assert!(
            host.socket(&format!("udp://{}", udp.local_addr()?))
                .is_listening()?
        );
    }
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("my socket [*]");
    let listener = UnixListener::bind(&path)?;
    let spec = format!("unix://{}", path.display());
    assert!(host.socket(&spec).is_listening()?);
    drop(listener);
    assert!(path.exists());
    assert!(!host.socket(&spec).is_listening()?);
    Ok(())
}

#[test]
fn native_kernel_resources() -> Result<()> {
    let host = Host::local()?;
    assert_eq!(host.sysctl("kernel.ostype").value()?, "Linux");
    assert!(host.sysctl("kernel.pid_max").value_as::<u64>()? > 0);
    assert!(host.mount_point("/").is_mounted()?);
    assert_eq!(host.mount_point("/proc").info()?.filesystem, "proc");
    assert_eq!(host.environment()?["LC_ALL"], "C");
    assert_eq!(host.environment()?["PATH"], std::env::var("PATH").unwrap());
    Ok(())
}
