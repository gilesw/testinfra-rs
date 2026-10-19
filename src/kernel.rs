use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use procfs_core::{FromBufRead, process::MountInfos};

use crate::{Error, Host, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountInfo {
    pub path: PathBuf,
    pub root: PathBuf,
    pub source: Option<String>,
    pub filesystem: String,
    pub options: BTreeMap<String, Option<String>>,
    pub super_options: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Copy)]
pub struct MountPoint<'a> {
    host: &'a Host,
    path: &'a str,
}

impl MountPoint<'_> {
    pub fn info(&self) -> Result<MountInfo> {
        self.lookup()?.ok_or_else(|| Error::NotFound {
            kind: "mount point",
            name: self.path.into(),
        })
    }
    pub fn is_mounted(&self) -> Result<bool> {
        Ok(self.lookup()?.is_some())
    }
    fn lookup(&self) -> Result<Option<MountInfo>> {
        if !self.path.starts_with('/') || self.path.contains('\0') {
            return Err(Error::InvalidInput(
                "mount point must be an absolute path without NUL".into(),
            ));
        }
        // Linux permits stacked mounts; the last matching entry is the visible one.
        Ok(self
            .host
            .mounts()?
            .into_iter()
            .rev()
            .find(|mount| mount.path == Path::new(self.path)))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Sysctl<'a> {
    host: &'a Host,
    key: &'a str,
}

impl Sysctl<'_> {
    fn path(&self) -> Result<String> {
        if self.key.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        }) {
            return Err(Error::InvalidInput(
                "sysctl requires a dotted key of letters, digits, underscores or hyphens".into(),
            ));
        }
        Ok(format!("/proc/sys/{}", self.key.replace('.', "/")))
    }
    pub fn exists(&self) -> Result<bool> {
        self.host.file(&self.path()?).exists()
    }
    pub fn value(&self) -> Result<String> {
        Ok(self
            .host
            .file(&self.path()?)
            .content()?
            .trim_end_matches('\n')
            .into())
    }
    pub fn value_as<T: FromStr>(&self) -> Result<T> {
        let value = self.value()?;
        value
            .parse()
            .map_err(|_| Error::Parse(format!("cannot parse sysctl {} value {value:?}", self.key)))
    }
}

impl Host {
    /// Environment inherited by an inspection process, including LC_ALL=C.
    pub fn environment(&self) -> Result<BTreeMap<String, String>> {
        let result = self.exec("env", &["-0"])?;
        if !result.success() {
            return Err(Error::CommandFailed {
                command: "env -0".into(),
                result,
            });
        }
        parse_environment(result.stdout_text()?)
    }
    pub fn sysctl<'a>(&'a self, key: &'a str) -> Sysctl<'a> {
        Sysctl { host: self, key }
    }
    pub fn mount_point<'a>(&'a self, path: &'a str) -> MountPoint<'a> {
        MountPoint { host: self, path }
    }
    /// Mounts in the inspected process's mount namespace.
    pub fn mounts(&self) -> Result<Vec<MountInfo>> {
        parse_mounts(&self.file("/proc/self/mountinfo").content_bytes()?)
    }
}

fn parse_environment(text: &str) -> Result<BTreeMap<String, String>> {
    text.split_terminator('\0')
        .map(|entry| {
            let (key, value) = entry
                .split_once('=')
                .ok_or_else(|| Error::Parse("environment entry has no '=' separator".into()))?;
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn parse_mounts(bytes: &[u8]) -> Result<Vec<MountInfo>> {
    let mounts = MountInfos::from_buf_read(bytes).map_err(|e| Error::Parse(e.to_string()))?;
    mounts
        .into_iter()
        .map(|mount| {
            let decode =
                |value: &str| unescaper::unescape(value).map_err(|e| Error::Parse(e.to_string()));
            Ok(MountInfo {
                path: PathBuf::from(decode(
                    mount
                        .mount_point
                        .to_str()
                        .ok_or_else(|| Error::Parse("non-UTF-8 mount path".into()))?,
                )?),
                root: PathBuf::from(decode(&mount.root)?),
                source: mount.mount_source.map(|s| decode(&s)).transpose()?,
                filesystem: mount.fs_type,
                options: mount.mount_options.into_iter().collect(),
                super_options: mount.super_options.into_iter().collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_preserves_values() -> Result<()> {
        let values = parse_environment("EMPTY=\0LINES=one\ntwo=three\0")?;
        assert_eq!(values["EMPTY"], "");
        assert_eq!(values["LINES"], "one\ntwo=three");
        assert!(parse_environment("broken\0").is_err());
        Ok(())
    }
    #[test]
    fn mount_paths_decode_once_and_keep_options() -> Result<()> {
        let mounts = parse_mounts(br"24 1 0:20 /root\134040 /mnt\040data\011tab\012line rw,nosuid shared:1 - tmpfs source\040name rw,size=1024k
")?;
        let mount = &mounts[0];
        assert_eq!(mount.path, Path::new("/mnt data\ttab\nline"));
        assert_eq!(mount.root, Path::new(r"/root\040"));
        assert_eq!(mount.source.as_deref(), Some("source name"));
        assert_eq!(mount.super_options["size"].as_deref(), Some("1024k"));
        assert!(parse_mounts(b"invalid\n").is_err());
        Ok(())
    }
}
