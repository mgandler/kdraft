//! Locating `kicad-cli` and reading its version.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::SourceError;
use crate::runner::CommandRunner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// Parses the first `MAJOR.MINOR[.PATCH]` in `text`, e.g. from
    /// `kicad-cli version` output (`9.0.6`, `10.0.0-rc1`,
    /// `9.99.0-1234-gabcdef`). A missing patch is 0.
    pub fn parse(text: &str) -> Option<Version> {
        text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
            .find_map(|tok| {
                let tok = tok.trim_start_matches(['v', 'V']);
                let mut parts = tok.split(['.', '-', '+']);
                let major = parts.next()?.parse().ok()?;
                let minor = parts.next()?.parse().ok()?;
                let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
                Some(Version::new(major, minor, patch))
            })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Optional `kicad-cli` features that depend on the KiCad version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// `--variant` on export commands (KiCad 10).
    Variant,
}

impl Capability {
    pub fn min_version(self) -> (Version, &'static str) {
        match self {
            Capability::Variant => (Version::new(10, 0, 0), "10.0"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Capability::Variant => "--variant",
        }
    }
}

/// A located `kicad-cli` with its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KicadCli {
    pub path: PathBuf,
    pub version: Version,
    /// Trimmed output of `kicad-cli version`; part of cache keys.
    pub version_string: String,
    /// Extra `--define-var NAME=VALUE` for the commands that accept them
    /// (from the user settings).
    pub vars: Vec<(String, String)>,
}

impl KicadCli {
    pub fn supports(&self, cap: Capability) -> bool {
        self.version >= cap.min_version().0
    }

    pub fn require(&self, cap: Capability) -> Result<(), SourceError> {
        if self.supports(cap) {
            Ok(())
        } else {
            Err(SourceError::Unsupported {
                feature: cap.name(),
                needs: cap.min_version().1,
                have: self.version.to_string(),
            })
        }
    }

    /// Runs `kicad-cli version` and parses the result.
    pub fn probe(path: &Path, runner: &dyn CommandRunner) -> Result<KicadCli, SourceError> {
        let out = runner.run(path, &[OsString::from("version")])?;
        let text = out.stdout_text().trim().to_string();
        if !out.success() {
            return Err(SourceError::CommandFailed {
                program: path.to_path_buf(),
                args: "version".to_string(),
                status: out.status_text(),
                stderr: out.stderr_text(),
            });
        }
        let version = Version::parse(&text).ok_or(SourceError::CliVersion {
            output: text.clone(),
        })?;
        Ok(KicadCli {
            path: path.to_path_buf(),
            version,
            version_string: text,
            vars: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// What discovery needs from the system, so tests can fake it.
pub trait Environment {
    fn var(&self, name: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
    /// Names of the entries of a directory (empty if unreadable).
    fn dir_names(&self, path: &Path) -> Vec<String>;
    fn os(&self) -> Os;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn dir_names(&self, path: &Path) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn os(&self) -> Os {
        Os::current()
    }
}

pub const ENV_VAR: &str = "KDRAFT_KICAD_CLI";

/// Candidate locations in search order, after `KDRAFT_KICAD_CLI`: each
/// `PATH` entry, then the platform's standard install locations.
pub fn candidates(env: &dyn Environment) -> Vec<PathBuf> {
    let os = env.os();
    let exe = if os == Os::Windows {
        "kicad-cli.exe"
    } else {
        "kicad-cli"
    };
    let mut out = Vec::new();
    if let Some(path) = env.var("PATH") {
        let path = path.to_string_lossy().into_owned();
        let sep = if os == Os::Windows { ';' } else { ':' };
        for dir in path.split(sep).filter(|d| !d.is_empty()) {
            out.push(join(os, dir, exe));
        }
    }
    match os {
        Os::Windows => {
            let mut bases = Vec::new();
            for var in ["ProgramFiles", "ProgramW6432"] {
                if let Some(v) = env.var(var) {
                    bases.push(v.to_string_lossy().into_owned());
                }
            }
            bases.push(r"C:\Program Files".to_string());
            bases.dedup();
            for base in bases {
                let kicad = join(os, &base, "KiCad");
                // Version folders such as 9.0 and 10.0, newest first.
                let mut versions: Vec<(Version, String)> = env
                    .dir_names(&kicad)
                    .into_iter()
                    .filter_map(|n| Version::parse(&n).map(|v| (v, n)))
                    .collect();
                versions.sort_by_key(|v| std::cmp::Reverse(v.0));
                for (_, name) in versions {
                    let bin = format!("{}\\{name}\\bin", kicad.to_string_lossy());
                    out.push(join(os, &bin, exe));
                }
            }
        }
        Os::MacOs => {
            out.push(PathBuf::from(
                "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli",
            ));
            out.push(PathBuf::from(
                "/Applications/KiCad.app/Contents/MacOS/kicad-cli",
            ));
        }
        Os::Linux => {
            out.push(PathBuf::from("/usr/bin/kicad-cli"));
            out.push(PathBuf::from("/usr/local/bin/kicad-cli"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// Joins path parts with the target OS's separator, independent of the
/// host, so Windows paths can be tested on Linux.
fn join(os: Os, dir: &str, name: &str) -> PathBuf {
    let sep = if os == Os::Windows { '\\' } else { '/' };
    let dir = dir.trim_end_matches(['/', '\\']);
    PathBuf::from(format!("{dir}{sep}{name}"))
}

/// Finds `kicad-cli` as [`resolve`] does with the setting "auto":
/// `KDRAFT_KICAD_CLI` if set (it must exist), else the newest install.
pub fn discover(
    env: &dyn Environment,
    runner: &dyn CommandRunner,
) -> Result<KicadCli, SourceError> {
    resolve(None, &crate::settings::CliChoice::Auto, env, runner).map(|r| r.0)
}

/// An installed kicad-cli found by [`list_installs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub path: PathBuf,
    /// `None` when `kicad-cli version` failed (see `error`).
    pub cli: Option<KicadCli>,
    pub error: Option<String>,
}

/// Every existing [`candidates`] entry, probed for its version.
pub fn list_installs(env: &dyn Environment, runner: &dyn CommandRunner) -> Vec<Install> {
    candidates(env)
        .into_iter()
        .filter(|p| env.is_file(p))
        .map(|p| match KicadCli::probe(&p, runner) {
            Ok(c) => Install {
                path: p,
                cli: Some(c),
                error: None,
            },
            Err(e) => Install {
                path: p,
                cli: None,
                error: Some(e.to_string()),
            },
        })
        .collect()
}

/// Where the kicad-cli in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliSource {
    /// `--kicad-cli` on the command line.
    Flag,
    /// `KDRAFT_KICAD_CLI`.
    Env,
    /// The `kicad_cli` user setting.
    Setting,
    /// The newest install found.
    Auto,
}

impl CliSource {
    pub fn label(self) -> &'static str {
        match self {
            CliSource::Flag => "command line",
            CliSource::Env => ENV_VAR,
            CliSource::Setting => "settings",
            CliSource::Auto => "auto",
        }
    }
}

/// The kicad-cli to use: the command-line `flag`, then `KDRAFT_KICAD_CLI`,
/// then the `setting`, then the newest install found.
pub fn resolve(
    flag: Option<&Path>,
    setting: &crate::settings::CliChoice,
    env: &dyn Environment,
    runner: &dyn CommandRunner,
) -> Result<(KicadCli, CliSource), SourceError> {
    if let Some(p) = flag {
        if !env.is_file(p) {
            return Err(SourceError::CliFlagInvalid(p.to_path_buf()));
        }
        return Ok((KicadCli::probe(p, runner)?, CliSource::Flag));
    }
    if let Some(p) = env.var(ENV_VAR).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if !env.is_file(&p) {
            return Err(SourceError::CliEnvInvalid(p));
        }
        return Ok((KicadCli::probe(&p, runner)?, CliSource::Env));
    }
    if let crate::settings::CliChoice::Path(p) = setting {
        if !env.is_file(p) {
            return Err(SourceError::CliSettingInvalid(p.clone()));
        }
        return Ok((KicadCli::probe(p, runner)?, CliSource::Setting));
    }
    let installs = list_installs(env, runner);
    // Newest version; the first found among equals.
    let mut best: Option<KicadCli> = None;
    for i in &installs {
        if let Some(c) = &i.cli {
            if best.as_ref().is_none_or(|b| c.version > b.version) {
                best = Some(c.clone());
            }
        }
    }
    match best {
        Some(c) => Ok((c, CliSource::Auto)),
        // None usable: the first one's own error.
        None => match installs.first() {
            Some(i) => KicadCli::probe(&i.path, runner).map(|c| (c, CliSource::Auto)),
            None => Err(SourceError::CliNotFound {
                searched: candidates(env),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::runner::{CommandOutput, FakeRunner};

    struct FakeEnv {
        os: Os,
        vars: HashMap<String, String>,
        files: HashSet<PathBuf>,
        dirs: HashMap<PathBuf, Vec<String>>,
    }

    impl FakeEnv {
        fn new(os: Os) -> Self {
            FakeEnv {
                os,
                vars: HashMap::new(),
                files: HashSet::new(),
                dirs: HashMap::new(),
            }
        }
    }

    impl Environment for FakeEnv {
        fn var(&self, name: &str) -> Option<OsString> {
            self.vars.get(name).map(OsString::from)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }
        fn dir_names(&self, path: &Path) -> Vec<String> {
            self.dirs.get(path).cloned().unwrap_or_default()
        }
        fn os(&self) -> Os {
            self.os
        }
    }

    fn version_runner(out: &'static str) -> FakeRunner {
        FakeRunner::new(move |_, args| {
            assert_eq!(args, ["version"]);
            CommandOutput::ok(out)
        })
    }

    #[test]
    fn precedence_and_multiple_installs() {
        use crate::settings::CliChoice;
        let mut env = FakeEnv::new(Os::Windows);
        env.vars
            .insert("ProgramFiles".into(), r"C:\Program Files".into());
        env.dirs.insert(
            PathBuf::from(r"C:\Program Files\KiCad"),
            vec!["9.0".into(), "10.0".into()],
        );
        let k9 = PathBuf::from(r"C:\Program Files\KiCad\9.0\bin\kicad-cli.exe");
        let k10 = PathBuf::from(r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe");
        let mine = PathBuf::from(r"D:\kc\kicad-cli.exe");
        let flag = PathBuf::from(r"E:\x\kicad-cli.exe");
        for p in [&k9, &k10, &mine, &flag] {
            env.files.insert(p.clone());
        }
        let runner = FakeRunner::new(|prog, _| {
            let p = prog.to_string_lossy();
            CommandOutput::ok(if p.contains("9.0") {
                "9.0.6"
            } else if p.contains("10.0") {
                "10.0.8"
            } else if p.contains("kc") {
                "9.0.1"
            } else {
                "8.0.0"
            })
        });
        let installs = list_installs(&env, &runner);
        let vs: Vec<String> = installs
            .iter()
            .map(|i| i.cli.as_ref().unwrap().version.to_string())
            .collect();
        assert_eq!(vs, ["10.0.8", "9.0.6"]);
        // Auto: the newest.
        let (c, src) = resolve(None, &CliChoice::Auto, &env, &runner).unwrap();
        assert_eq!((c.path.clone(), src), (k10.clone(), CliSource::Auto));
        // The setting beats auto.
        let set = CliChoice::Path(mine.clone());
        let (c, src) = resolve(None, &set, &env, &runner).unwrap();
        assert_eq!((c.path.clone(), src), (mine.clone(), CliSource::Setting));
        // The environment beats the setting.
        env.vars
            .insert(ENV_VAR.into(), k9.to_string_lossy().into_owned());
        let (c, src) = resolve(None, &set, &env, &runner).unwrap();
        assert_eq!((c.path.clone(), src), (k9.clone(), CliSource::Env));
        // The flag beats everything.
        let (c, src) = resolve(Some(&flag), &set, &env, &runner).unwrap();
        assert_eq!((c.path, src), (flag, CliSource::Flag));
        // A setting that points nowhere is an error with advice.
        env.vars.remove(ENV_VAR);
        let gone = CliChoice::Path(PathBuf::from(r"Z:\gone.exe"));
        let e = resolve(None, &gone, &env, &runner).unwrap_err();
        assert!(matches!(e, SourceError::CliSettingInvalid(_)), "{e}");
    }

    #[test]
    fn version_parsing() {
        assert_eq!(Version::parse("9.0.6\n"), Some(Version::new(9, 0, 6)));
        assert_eq!(Version::parse("10.0.0-rc1"), Some(Version::new(10, 0, 0)));
        assert_eq!(
            Version::parse("9.99.0-1234-gabcdef"),
            Some(Version::new(9, 99, 0))
        );
        assert_eq!(Version::parse("8.0"), Some(Version::new(8, 0, 0)));
        assert_eq!(
            Version::parse("KiCad 9.0.1 (release)"),
            Some(Version::new(9, 0, 1))
        );
        assert_eq!(Version::parse("garbage"), None);
        assert_eq!(Version::parse(""), None);
        assert!(Version::new(10, 0, 0) > Version::new(9, 99, 9));
    }

    #[test]
    fn env_var_wins_and_must_exist() {
        let mut env = FakeEnv::new(Os::Linux);
        env.vars.insert(ENV_VAR.into(), "/opt/kc/kicad-cli".into());
        env.vars.insert("PATH".into(), "/usr/bin".into());
        env.files.insert("/opt/kc/kicad-cli".into());
        env.files.insert("/usr/bin/kicad-cli".into());
        let cli = discover(&env, &version_runner("10.0.1\n")).unwrap();
        assert_eq!(cli.path, PathBuf::from("/opt/kc/kicad-cli"));
        assert_eq!(cli.version, Version::new(10, 0, 1));
        assert_eq!(cli.version_string, "10.0.1");
        assert!(cli.supports(Capability::Variant));

        env.files.remove(Path::new("/opt/kc/kicad-cli"));
        assert!(matches!(
            discover(&env, &version_runner("10.0.1")),
            Err(SourceError::CliEnvInvalid(_))
        ));
    }

    #[test]
    fn path_then_standard_locations_linux() {
        let mut env = FakeEnv::new(Os::Linux);
        env.vars
            .insert("PATH".into(), "/home/u/bin::/usr/bin".into());
        env.files.insert("/usr/bin/kicad-cli".into());
        let c = candidates(&env);
        assert_eq!(c[0], PathBuf::from("/home/u/bin/kicad-cli"));
        assert_eq!(c[1], PathBuf::from("/usr/bin/kicad-cli"));
        // /usr/bin appears once even though it is also a standard location.
        assert_eq!(
            c.iter()
                .filter(|p| p.as_path() == Path::new("/usr/bin/kicad-cli"))
                .count(),
            1
        );
        let cli = discover(&env, &version_runner("9.0.6")).unwrap();
        assert_eq!(cli.path, PathBuf::from("/usr/bin/kicad-cli"));
        assert!(!cli.supports(Capability::Variant));
        assert!(matches!(
            cli.require(Capability::Variant),
            Err(SourceError::Unsupported { .. })
        ));
    }

    #[test]
    fn windows_program_files_newest_first() {
        let mut env = FakeEnv::new(Os::Windows);
        env.vars
            .insert("PATH".into(), r"C:\Windows;C:\tools\".into());
        env.dirs.insert(
            PathBuf::from(r"C:\Program Files\KiCad"),
            vec!["9.0".into(), "10.0".into(), "readme.txt".into()],
        );
        env.files.insert(PathBuf::from(
            r"C:\Program Files\KiCad\9.0\bin\kicad-cli.exe",
        ));
        env.files.insert(PathBuf::from(
            r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe",
        ));
        let c = candidates(&env);
        assert_eq!(c[0], PathBuf::from(r"C:\Windows\kicad-cli.exe"));
        assert_eq!(c[1], PathBuf::from(r"C:\tools\kicad-cli.exe"));
        assert_eq!(
            c[2],
            PathBuf::from(r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe")
        );
        assert_eq!(
            c[3],
            PathBuf::from(r"C:\Program Files\KiCad\9.0\bin\kicad-cli.exe")
        );
        let cli = discover(&env, &version_runner("10.0.0")).unwrap();
        assert_eq!(
            cli.path,
            PathBuf::from(r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe")
        );
    }

    #[test]
    fn macos_app_bundle() {
        let mut env = FakeEnv::new(Os::MacOs);
        env.files
            .insert("/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli".into());
        let cli = discover(&env, &version_runner("9.0.2")).unwrap();
        assert_eq!(cli.version, Version::new(9, 0, 2));
    }

    #[test]
    fn not_found_and_bad_version() {
        let env = FakeEnv::new(Os::Linux);
        match discover(&env, &version_runner("9.0.0")) {
            Err(SourceError::CliNotFound { searched }) => {
                assert_eq!(
                    searched,
                    [
                        PathBuf::from("/usr/bin/kicad-cli"),
                        PathBuf::from("/usr/local/bin/kicad-cli")
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
        let mut env = FakeEnv::new(Os::Linux);
        env.files.insert("/usr/bin/kicad-cli".into());
        assert!(matches!(
            discover(&env, &version_runner("oops")),
            Err(SourceError::CliVersion { .. })
        ));
        let failing = FakeRunner::new(|_, _| CommandOutput::failed(1, "boom"));
        assert!(matches!(
            discover(&env, &failing),
            Err(SourceError::CommandFailed { .. })
        ));
    }
}
