use crate::{Error, Host, Result, host::quote};

macro_rules! resource {
    ($name:ident, $method:ident) => {
        #[derive(Debug, Clone, Copy)]
        pub struct $name<'a> {
            host: &'a Host,
            name: &'a str,
        }
        impl Host {
            pub fn $method<'a>(&'a self, name: &'a str) -> $name<'a> {
                $name { host: self, name }
            }
        }
    };
}

resource!(File, file);
resource!(Package, package);
resource!(User, user);
resource!(Group, group);
resource!(Process, process);

/// Metadata follows symbolic links, as do content reads and file/directory checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub user: String,
    pub group: String,
}

impl File<'_> {
    fn path(&self) -> Result<String> {
        if self.name.is_empty() {
            return Err(Error::InvalidInput("file path cannot be empty".into()));
        }
        // Prefix relative paths so even a path beginning with '-' is literal.
        quote(&if self.name.starts_with('/') {
            self.name.into()
        } else {
            format!("./{}", self.name)
        })
    }

    /// Includes dangling symbolic links.
    pub fn exists(&self) -> Result<bool> {
        let p = self.path()?;
        self.host.predicate(&format!("test -e {p} || test -L {p}"))
    }
    pub fn is_file(&self) -> Result<bool> {
        self.test("-f")
    }
    pub fn is_directory(&self) -> Result<bool> {
        self.test("-d")
    }
    pub fn is_symlink(&self) -> Result<bool> {
        self.test("-L")
    }
    /// Whether the executing user can execute the file or search the directory.
    pub fn is_executable(&self) -> Result<bool> {
        self.test("-x")
    }
    /// Return the link's stored target, without resolving it or stripping newlines.
    pub fn link_target(&self) -> Result<String> {
        self.host
            .check_output(&format!("readlink -n {}", self.path()?))
    }
    fn test(&self, flag: &str) -> Result<bool> {
        self.host
            .predicate(&format!("test {flag} {}", self.path()?))
    }

    pub fn content_bytes(&self) -> Result<Vec<u8>> {
        let script = format!("cat {}", self.path()?);
        let result = self.host.run(&script)?;
        if !result.success() {
            return Err(Error::CommandFailed {
                command: script,
                result,
            });
        }
        Ok(result.stdout)
    }
    pub fn content(&self) -> Result<String> {
        String::from_utf8(self.content_bytes()?).map_err(|e| Error::Parse(e.to_string()))
    }
    /// Match a Rust regular expression. Use `(?m)` for line anchors.
    pub fn contains(&self, pattern: &str) -> Result<bool> {
        let regex = regex::Regex::new(pattern).map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(regex.is_match(&self.content()?))
    }
    pub fn contains_literal(&self, text: &str) -> Result<bool> {
        Ok(self.content()?.contains(text))
    }

    pub fn metadata(&self) -> Result<FileMetadata> {
        let output = self.host.check_output(&format!(
            "stat -L -c '%a\n%u\n%g\n%s\n%U\n%G' {}",
            self.path()?
        ))?;
        let fields: Vec<_> = output.lines().collect();
        if fields.len() != 6 {
            return Err(Error::Parse(format!("unexpected stat output: {output:?}")));
        }
        Ok(FileMetadata {
            mode: u32::from_str_radix(fields[0], 8).map_err(|e| Error::Parse(e.to_string()))?,
            uid: number(fields[1])?,
            gid: number(fields[2])?,
            size: number(fields[3])?,
            user: fields[4].into(),
            group: fields[5].into(),
        })
    }
    pub fn mode(&self) -> Result<u32> {
        Ok(self.metadata()?.mode)
    }
    pub fn uid(&self) -> Result<u32> {
        Ok(self.metadata()?.uid)
    }
    pub fn gid(&self) -> Result<u32> {
        Ok(self.metadata()?.gid)
    }
    pub fn size(&self) -> Result<u64> {
        Ok(self.metadata()?.size)
    }
    pub fn user(&self) -> Result<String> {
        Ok(self.metadata()?.user)
    }
    pub fn group(&self) -> Result<String> {
        Ok(self.metadata()?.group)
    }
}

impl Package<'_> {
    /// Support dpkg (Debian/Ubuntu), rpm (Fedora/RHEL/SUSE), and apk (Alpine).
    /// Returns None only when the selected package manager reports absence.
    pub fn version(&self) -> Result<Option<String>> {
        let name = identifier(self.name)?;
        let script = format!(
            r#"
if command -v dpkg-query >/dev/null 2>&1; then
    value=$(dpkg-query -W -f='${{Status}}\t${{Version}}' -- {name}); rc=$?
    if [ "$rc" -eq 1 ]; then exit 1; fi
    if [ "$rc" -ne 0 ]; then exit "$rc"; fi
    case "$value" in 'install ok installed'*) printf '%s\n' "${{value#*$(printf '\t')}}";; *) exit 1;; esac
elif command -v rpm >/dev/null 2>&1; then
    rpm -q --qf '%{{VERSION}}-%{{RELEASE}}\n' -- {name}
elif command -v apk >/dev/null 2>&1; then
    apk info -e -- {name} >/dev/null; rc=$?
    if [ "$rc" -ne 0 ]; then exit "$rc"; fi
    value=$(apk info -v -- {name}) || exit 2
    name={name}; printf '%s\n' "${{value#"$name"-}}"
else
    echo 'supported package manager (dpkg, rpm, apk) not found' >&2; exit 127
fi"#
        );
        optional_output(self.host, &script)
    }
    pub fn is_installed(&self) -> Result<bool> {
        Ok(self.version()?.is_some())
    }
}

pub(crate) fn identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.+:@$".contains(&b))
    {
        return Err(Error::InvalidInput(format!(
            "invalid resource name: {value:?}"
        )));
    }
    quote(value)
}

fn number<T: std::str::FromStr>(value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| Error::Parse(format!("expected a number, got {value:?}")))
}

fn optional_output(host: &Host, script: &str) -> Result<Option<String>> {
    let result = host.run(script)?;
    match result.exit_status {
        0 => Ok(Some(result.stdout_text()?.trim_end_matches('\n').into())),
        1 => Ok(None),
        _ => Err(Error::CommandFailed {
            command: script.into(),
            result,
        }),
    }
}

fn account(host: &Host, database: &str, name: &str) -> Result<Option<Vec<String>>> {
    let name = identifier(name)?;
    let script = format!(
        r#"
if command -v getent >/dev/null 2>&1; then
    getent {database} {name}; rc=$?
    if [ "$rc" -eq 2 ]; then exit 1; fi
    if [ "$rc" -ne 0 ]; then exit 2; fi
else
    awk -F: -v key={name} '$1 == key || $3 == key {{ print; found=1; exit }} END {{ exit !found }}' /etc/{database}
fi"#
    );
    optional_output(host, &script)?
        .map(|line| {
            let fields: Vec<String> = line.split(':').map(str::to_owned).collect();
            let expected = if database == "passwd" { 7 } else { 4 };
            if fields.len() != expected {
                return Err(Error::Parse(format!(
                    "unexpected {database} entry: {line:?}"
                )));
            }
            Ok(fields)
        })
        .transpose()
}

impl User<'_> {
    fn entry(&self) -> Result<Vec<String>> {
        account(self.host, "passwd", self.name)?.ok_or_else(|| Error::NotFound {
            kind: "user",
            name: self.name.into(),
        })
    }
    pub fn exists(&self) -> Result<bool> {
        Ok(account(self.host, "passwd", self.name)?.is_some())
    }
    pub fn uid(&self) -> Result<u32> {
        number(&self.entry()?[2])
    }
    pub fn gid(&self) -> Result<u32> {
        number(&self.entry()?[3])
    }
    pub fn home(&self) -> Result<String> {
        Ok(self.entry()?[5].clone())
    }
    pub fn shell(&self) -> Result<String> {
        Ok(self.entry()?[6].clone())
    }
    /// Includes both primary and supplementary groups.
    pub fn groups(&self) -> Result<Vec<String>> {
        // Resolve numeric IDs to names before invoking id (id's numeric handling varies).
        let name = identifier(&self.entry()?[0])?;
        Ok(self
            .host
            .check_output(&format!("id -Gn {name}"))?
            .split_whitespace()
            .map(str::to_owned)
            .collect())
    }
}

impl Group<'_> {
    fn entry(&self) -> Result<Vec<String>> {
        account(self.host, "group", self.name)?.ok_or_else(|| Error::NotFound {
            kind: "group",
            name: self.name.into(),
        })
    }
    pub fn exists(&self) -> Result<bool> {
        Ok(account(self.host, "group", self.name)?.is_some())
    }
    pub fn gid(&self) -> Result<u32> {
        number(&self.entry()?[2])
    }
    /// Explicit supplementary members from the group database (excludes primary members).
    pub fn members(&self) -> Result<Vec<String>> {
        Ok(self.entry()?[3]
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

impl Process<'_> {
    /// Match the complete process name using pgrep's extended regex syntax.
    /// This is the kernel process name (often limited to 15 characters), not argv.
    pub fn pids(&self) -> Result<Vec<u32>> {
        let pattern = quote(self.name)?;
        let script = format!("pgrep -x -- {pattern}");
        let result = self.host.run(&script)?;
        match result.exit_status {
            0 => result.stdout_text()?.lines().map(number).collect(),
            // BusyBox reports regex syntax errors with status 1, unlike procps.
            1 if result.stderr.is_empty() => Ok(Vec::new()),
            _ => Err(Error::CommandFailed {
                command: script,
                result,
            }),
        }
    }
    pub fn is_running(&self) -> Result<bool> {
        Ok(!self.pids()?.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub distribution: String,
    pub release: Option<String>,
    pub architecture: String,
    pub kernel: String,
}

impl Host {
    pub fn system_info(&self) -> Result<SystemInfo> {
        let text = self.check_output("cat /etc/os-release")?;
        let release = etc_os_release::OsRelease::from_reader(text.as_bytes())
            .map_err(|e| Error::Parse(e.to_string()))?;
        Ok(SystemInfo {
            distribution: release.id().to_owned(),
            release: release.version_id().map(str::to_owned),
            architecture: self.check_output("uname -m")?.trim().into(),
            kernel: self.check_output("uname -r")?.trim().into(),
        })
    }
}
