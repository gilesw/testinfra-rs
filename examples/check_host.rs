use testinfra::{Host, Result};

fn main() -> Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "local://".into());
    let host = Host::from_url(&url)?;
    assert!(host.file("/etc/passwd").is_file()?);
    assert!(host.file("/etc/passwd").contains("(?m)^root:")?);
    assert_eq!(host.user("root").uid()?, 0);
    assert_eq!(host.sysctl("kernel.ostype").value()?, "Linux");
    assert!(host.mount_point("/").is_mounted()?);
    println!("Checks passed for {url}: {:?}", host.system_info()?);
    Ok(())
}
