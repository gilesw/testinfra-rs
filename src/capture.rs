use std::fmt;

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{Error, Result};

/// The output stream whose capture limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl fmt::Display for OutputStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputLimits {
    pub stdout: usize,
    pub stderr: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            stdout: 4 * 1024 * 1024,
            stderr: 4 * 1024 * 1024,
        }
    }
}

pub(crate) fn append(
    captured: &mut Vec<u8>,
    bytes: &[u8],
    limit: usize,
    stream: OutputStream,
) -> Result<()> {
    if bytes.len() > limit.saturating_sub(captured.len()) {
        return Err(Error::OutputLimitExceeded { stream, limit });
    }
    captured.extend_from_slice(bytes);
    Ok(())
}

pub(crate) async fn read(
    mut reader: impl AsyncRead + Unpin,
    limit: usize,
    stream: OutputStream,
) -> Result<Vec<u8>> {
    let mut captured = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        // Read at most one byte beyond the remaining allowance to detect overflow.
        let length = buffer.len().min((limit - captured.len()).saturating_add(1));
        let count = reader.read(&mut buffer[..length]).await?;
        if count == 0 {
            return Ok(captured);
        }
        append(&mut captured, &buffer[..count], limit, stream)?;
    }
}
