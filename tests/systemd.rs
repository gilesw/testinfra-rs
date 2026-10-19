#![cfg(target_os = "linux")]

use std::{env, process};
use testinfra::{Host, Result};

#[test]
#[ignore = "requires running systemd and fixture units; use tests/fixtures/systemd.sh"]
fn live_systemd_states() -> Result<()> {
    let active = env::var("TESTINFRA_SYSTEMD_ACTIVE_UNIT")
        .expect("set TESTINFRA_SYSTEMD_ACTIVE_UNIT to an active, enabled fixture");
    let inactive = env::var("TESTINFRA_SYSTEMD_INACTIVE_UNIT")
        .expect("set TESTINFRA_SYSTEMD_INACTIVE_UNIT to an inactive, disabled fixture");
    let host = Host::local()?;
    assert!(host.service(&active).is_running()?);
    assert!(host.service(&active).is_enabled()?);
    assert!(!host.service(&inactive).is_running()?);
    assert!(!host.service(&inactive).is_enabled()?);
    let missing = format!("testinfra-no-such-unit-{}.service", process::id());
    assert!(!host.service(&missing).is_running()?);
    assert!(!host.service(&missing).is_enabled()?);
    Ok(())
}
