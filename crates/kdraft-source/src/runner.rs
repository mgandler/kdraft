//! Running external programs behind a trait, so tests never need KiCad.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::SourceError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOutput {
    /// Exit code; `None` if the process was terminated by a signal.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn ok(stdout: impl Into<Vec<u8>>) -> CommandOutput {
        CommandOutput {
            status: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
        }
    }

    pub fn failed(code: i32, stderr: impl Into<Vec<u8>>) -> CommandOutput {
        CommandOutput {
            status: Some(code),
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }

    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    pub fn status_text(&self) -> String {
        match self.status {
            Some(c) => format!("exit code {c}"),
            None => "terminated by signal".to_string(),
        }
    }
}

pub trait CommandRunner: Send + Sync {
    /// Runs `program` with `args` to completion. An error means the program
    /// could not be started; a non-zero exit is reported in the output.
    fn run(&self, program: &Path, args: &[OsString]) -> Result<CommandOutput, SourceError>;
}

/// Runs programs with `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &Path, args: &[OsString]) -> Result<CommandOutput, SourceError> {
        let out = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|source| SourceError::Spawn {
                program: program.to_path_buf(),
                source,
            })?;
        Ok(CommandOutput {
            status: out.status.code(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

/// One recorded call of a [`FakeRunner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub program: PathBuf,
    pub args: Vec<String>,
}

type Handler = dyn Fn(&Path, &[String]) -> CommandOutput + Send + Sync;

/// Test double: answers every call with a handler and records the calls.
/// The handler may write files (e.g. the `-o` output of an export).
pub struct FakeRunner {
    handler: Box<Handler>,
    calls: Mutex<Vec<Call>>,
}

impl FakeRunner {
    pub fn new(
        handler: impl Fn(&Path, &[String]) -> CommandOutput + Send + Sync + 'static,
    ) -> Self {
        FakeRunner {
            handler: Box::new(handler),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &Path, args: &[OsString]) -> Result<CommandOutput, SourceError> {
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if let Ok(mut c) = self.calls.lock() {
            c.push(Call {
                program: program.to_path_buf(),
                args: args.clone(),
            });
        }
        Ok((self.handler)(program, &args))
    }
}

/// The value following `flag` in an argument list.
pub fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn system_runner_captures_output_and_status() {
        let out = SystemRunner
            .run(
                Path::new("sh"),
                &["-c".into(), "echo out; echo err >&2; exit 3".into()],
            )
            .unwrap();
        assert_eq!(out.status, Some(3));
        assert_eq!(out.stdout_text(), "out\n");
        assert_eq!(out.stderr_text(), "err\n");
        assert!(!out.success());
    }

    #[test]
    fn system_runner_reports_spawn_failure() {
        let e = SystemRunner
            .run(Path::new("/nonexistent/kdraft-no-such-program"), &[])
            .unwrap_err();
        assert!(matches!(e, SourceError::Spawn { .. }));
    }

    #[test]
    fn fake_runner_records_calls() {
        let f = FakeRunner::new(|_, args| CommandOutput::ok(args.join(" ")));
        let out = f.run(Path::new("x"), &["a".into(), "b".into()]).unwrap();
        assert_eq!(out.stdout_text(), "a b");
        assert_eq!(
            f.calls(),
            vec![Call {
                program: "x".into(),
                args: vec!["a".into(), "b".into()]
            }]
        );
        let args = vec!["-o".to_string(), "f.svg".to_string()];
        assert_eq!(arg_value(&args, "-o"), Some("f.svg"));
        assert_eq!(arg_value(&args, "-x"), None);
    }
}
