use crate::{CommandResult, Error, Host, Result, resources::identifier};

#[derive(Debug, Clone, Copy)]
pub struct Service<'a> {
    host: &'a Host,
    name: &'a str,
}

impl Host {
    pub fn service<'a>(&'a self, name: &'a str) -> Service<'a> {
        Service { host: self, name }
    }
}

impl Service<'_> {
    fn state(&self, enabled: bool) -> Result<bool> {
        let name = identifier(self.name)?;
        if !self.host.predicate("if command -v systemctl >/dev/null 2>&1 && test -d /run/systemd/system; then exit 0; else exit 1; fi")? {
            return Err(Error::Unsupported("service checks require systemctl and a running systemd manager".into()));
        }
        let load_command = format!("systemctl show --property=LoadState --value -- {name}");
        let load = self.host.run(&load_command)?;
        if load.stdout_text()?.trim() == "not-found" {
            return Ok(false);
        }
        if !load.success() {
            return Err(Error::CommandFailed {
                command: load_command,
                result: load,
            });
        }
        if load.stdout_text()?.trim().is_empty() {
            return Err(Error::Parse("systemctl returned no LoadState".into()));
        }
        let operation = if enabled { "is-enabled" } else { "is-active" };
        let command = format!("systemctl {operation} -- {name}");
        interpret_state(&command, self.host.run(&command)?, enabled)
    }
    pub fn is_running(&self) -> Result<bool> {
        self.state(false)
    }
    /// Follows systemctl is-enabled semantics, including static/indirect units.
    pub fn is_enabled(&self) -> Result<bool> {
        self.state(true)
    }
}

fn interpret_state(command: &str, result: CommandResult, enabled: bool) -> Result<bool> {
    if result.success() {
        return Ok(true);
    }
    let state = result.stdout_text()?.trim();
    let absent = if enabled {
        matches!(
            state,
            "disabled" | "masked" | "masked-runtime" | "linked" | "linked-runtime" | "not-found"
        )
    } else {
        matches!(
            state,
            "inactive"
                | "failed"
                | "unknown"
                | "activating"
                | "deactivating"
                | "maintenance"
                | "not-found"
        )
    };
    if absent {
        Ok(false)
    } else {
        Err(Error::CommandFailed {
            command: command.into(),
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result(state: &str, status: i32) -> CommandResult {
        CommandResult {
            exit_status: status,
            stdout: format!("{state}\n").into_bytes(),
            stderr: String::new(),
        }
    }
    #[test]
    fn service_states_keep_failures_distinct() -> Result<()> {
        for state in [
            "enabled",
            "enabled-runtime",
            "static",
            "indirect",
            "generated",
            "transient",
            "alias",
        ] {
            assert!(interpret_state(
                "systemctl is-enabled",
                result(state, 0),
                true
            )?);
        }
        for state in [
            "disabled",
            "masked",
            "masked-runtime",
            "linked",
            "linked-runtime",
            "not-found",
        ] {
            assert!(!interpret_state(
                "systemctl is-enabled",
                result(state, 1),
                true
            )?);
        }
        assert!(interpret_state(
            "systemctl is-active",
            result("active", 0),
            false
        )?);
        for state in [
            "inactive",
            "failed",
            "activating",
            "deactivating",
            "maintenance",
            "unknown",
        ] {
            assert!(!interpret_state(
                "systemctl is-active",
                result(state, 3),
                false
            )?);
        }
        assert!(matches!(
            interpret_state("systemctl is-active", result("", 1), false),
            Err(Error::CommandFailed { .. })
        ));
        assert!(interpret_state("systemctl is-enabled", result("bad", 1), true).is_err());
        Ok(())
    }
}
