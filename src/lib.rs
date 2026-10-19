//! Testinfra-inspired Linux infrastructure tests using Rust's normal test runner.
//!
//! ```no_run
//! use testinfra::{Host, Result};
//!
//! #[test]
//! fn container_configuration() -> Result<()> {
//!     let host = Host::docker("web")?;
//!     assert!(host.file("/etc/passwd").contains(r"(?m)^root:")?);
//!     assert_eq!(host.user("root").uid()?, 0);
//!     Ok(())
//! }
//! ```

mod host;
mod kernel;
mod resources;
mod service;
mod socket;

pub use host::{CommandResult, Error, Host, Result};
pub use kernel::{MountInfo, MountPoint, Sysctl};
pub use resources::{File, FileMetadata, Group, Package, Process, SystemInfo, User};
pub use service::Service;
pub use socket::Socket;
