use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use crate::{
    select_conpty, ConPtyConfig, ConPtySelection, ConPtySource, ConPtyStatus, PtyError, Result,
    ShellInfo, ShellKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Starting,
    Running,
    Exiting,
    Exited,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExit {
    pub code: Option<u32>,
    pub state: SessionState,
}

#[derive(Debug, Clone)]
pub struct PtyCommand {
    pub shell: ShellInfo,
    pub working_directory: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PtySessionConfig {
    pub command: PtyCommand,
    pub columns: u16,
    pub rows: u16,
}

impl PtySessionConfig {
    pub fn validate(&self) -> Result<()> {
        if self.columns == 0
            || self.rows == 0
            || self.columns > i16::MAX as u16
            || self.rows > i16::MAX as u16
        {
            return Err(PtyError::InvalidSize {
                columns: self.columns,
                rows: self.rows,
            });
        }
        if self.command.shell.executable.as_os_str().is_empty() {
            return Err(PtyError::ShellNotFound("empty shell path".to_owned()));
        }
        Ok(())
    }
}

pub struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    state: SessionState,
    exit_code: Option<u32>,
    runtime: ConPtySelection,
}

const GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_millis(500);
const KILL_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

impl std::fmt::Debug for PtySession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PtySession")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl PtySession {
    pub fn spawn(config: &PtySessionConfig) -> Result<Self> {
        Self::spawn_with_runtime(config, &ConPtyConfig::default())
    }

    pub fn spawn_with_runtime(config: &PtySessionConfig, runtime: &ConPtyConfig) -> Result<Self> {
        config.validate()?;
        let selection = match select_conpty(runtime)? {
            ConPtyStatus::Selected(selection) => selection,
            ConPtyStatus::RejectedBundle { reason, .. } => ConPtySelection {
                reason,
                ..ConPtySelection::system()
            },
        };
        let bundled = selection.source == ConPtySource::Bundled;
        match Self::spawn_selected(config, selection) {
            Ok(session) => Ok(session),
            Err(_) if bundled => Self::spawn_selected(
                config,
                ConPtySelection {
                    reason: "bundled runtime failed to load or start; using system runtime"
                        .to_owned(),
                    ..ConPtySelection::system()
                },
            ),
            Err(error) => Err(error),
        }
    }

    fn spawn_selected(config: &PtySessionConfig, runtime: ConPtySelection) -> Result<Self> {
        #[cfg(windows)]
        let pty_system: Box<dyn portable_pty::PtySystem + Send> = match runtime.path.clone() {
            Some(path) => Box::new(portable_pty::win::conpty::ConPtySystem::with_conpty_dll(
                path,
            )),
            None => native_pty_system(),
        };
        #[cfg(not(windows))]
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: config.rows,
                cols: config.columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::Pty(error.to_string()))?;

        let executable = config
            .command
            .shell
            .executable
            .to_string_lossy()
            .into_owned();
        let mut command = CommandBuilder::new(executable);
        command.args(&config.command.shell.args);
        if let Some(directory) = config.command.working_directory.as_ref() {
            if matches!(config.command.shell.kind, ShellKind::Wsl { .. }) {
                command.arg("--cd");
                command.arg(directory);
            } else {
                command.cwd(directory);
            }
        }
        for (key, value) in &config.command.environment {
            command.env(key, value);
        }

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| PtyError::Pty(error.to_string()))?;
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PtyError::Pty(error.to_string()));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PtyError::Pty(error.to_string()));
            }
        };
        Ok(Self {
            master: Some(pair.master),
            reader: Some(reader),
            writer: Some(writer),
            child,
            state: SessionState::Running,
            exit_code: None,
            runtime,
        })
    }

    pub fn runtime(&self) -> &ConPtySelection {
        &self.runtime
    }

    pub fn into_backend(mut self) -> Result<capstan_terminal::BackendParts> {
        let reader = self.take_reader()?;
        let writer = self
            .writer
            .take()
            .ok_or_else(|| PtyError::Pty("session writer was transferred".to_owned()))?;
        Ok(capstan_terminal::BackendParts {
            reader,
            writer,
            control: std::sync::Arc::new(PtyBackendControl(std::sync::Mutex::new(self))),
        })
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if self.state != SessionState::Running {
            return Err(PtyError::Pty("session is not running".to_owned()));
        }
        self.reader
            .as_mut()
            .ok_or_else(|| PtyError::Pty("session reader was transferred".to_owned()))?
            .read(buffer)
            .map_err(PtyError::from)
    }

    pub fn take_reader(&mut self) -> Result<Box<dyn Read + Send>> {
        self.reader
            .take()
            .ok_or_else(|| PtyError::Pty("session reader was already transferred".to_owned()))
    }

    pub fn terminate(&mut self) -> Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        match self.child.kill() {
            Ok(()) => Ok(()),
            Err(error) => {
                if self.try_wait()?.is_some() {
                    Ok(())
                } else {
                    Err(PtyError::Pty(error.to_string()))
                }
            }
        }
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.state != SessionState::Running {
            return Err(PtyError::Pty("session is not running".to_owned()));
        }
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| PtyError::Pty("session writer was transferred".to_owned()))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(bytes.len())
    }

    pub fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        if columns == 0 || rows == 0 || columns > i16::MAX as u16 || rows > i16::MAX as u16 {
            return Err(PtyError::InvalidSize { columns, rows });
        }
        self.master
            .as_ref()
            .ok_or_else(|| PtyError::Pty("session is closed".to_owned()))?
            .resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::Pty(error.to_string()))
    }

    pub fn try_wait(&mut self) -> Result<Option<SessionExit>> {
        if self.state == SessionState::Exited {
            return Ok(Some(SessionExit {
                code: self.exit_code,
                state: SessionState::Exited,
            }));
        }
        let status = self
            .child
            .try_wait()
            .map_err(|error| PtyError::Pty(error.to_string()))?;
        let Some(status) = status else {
            return Ok(None);
        };
        Ok(Some(self.record_exit(status.exit_code())))
    }

    pub fn close(&mut self, force: bool) -> Result<SessionExit> {
        if self.state == SessionState::Exited {
            return Ok(SessionExit {
                code: self.exit_code,
                state: SessionState::Exited,
            });
        }
        if let Some(exit) = self.try_wait()? {
            return Ok(exit);
        }
        self.state = SessionState::Exiting;
        if force {
            self.kill_running_child()?;
            if self.state == SessionState::Exited {
                return Ok(SessionExit {
                    code: self.exit_code,
                    state: SessionState::Exited,
                });
            }
        } else {
            if let Some(writer) = self.writer.as_mut() {
                let _ = writer.write_all(b"exit\r\n");
                let _ = writer.flush();
            }
            if let Some(exit) = self.wait_for_exit(GRACEFUL_EXIT_TIMEOUT)? {
                return Ok(exit);
            }
            self.kill_running_child()?;
            if self.state == SessionState::Exited {
                return Ok(SessionExit {
                    code: self.exit_code,
                    state: SessionState::Exited,
                });
            }
        }
        self.wait_for_exit(KILL_EXIT_TIMEOUT)?
            .ok_or_else(|| PtyError::Pty("child process did not exit after termination".to_owned()))
    }

    fn kill_running_child(&mut self) -> Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        match self.child.kill() {
            Ok(()) => Ok(()),
            Err(error) => {
                if self.try_wait()?.is_some() {
                    Ok(())
                } else {
                    Err(PtyError::Pty(error.to_string()))
                }
            }
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Result<Option<SessionExit>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| PtyError::Pty(error.to_string()))?
            {
                return Ok(Some(self.record_exit(status.exit_code())));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(EXIT_POLL_INTERVAL);
        }
    }

    fn record_exit(&mut self, code: u32) -> SessionExit {
        self.state = SessionState::Exited;
        self.exit_code = Some(code);
        SessionExit {
            code: self.exit_code,
            state: SessionState::Exited,
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if self.state != SessionState::Exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct PtyBackendControl(std::sync::Mutex<PtySession>);
impl capstan_terminal::BackendControl for PtyBackendControl {
    fn resize(&self, columns: u16, rows: u16) -> capstan_terminal::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resize(columns, rows)
            .map_err(|error| capstan_terminal::TerminalError::Backend(error.code().to_owned()))
    }
    fn try_wait(&self) -> capstan_terminal::Result<Option<u32>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_wait()
            .map(|exit| exit.and_then(|exit| exit.code))
            .map_err(|error| capstan_terminal::TerminalError::Backend(error.code().to_owned()))
    }
    fn close(&self, terminate: bool) -> capstan_terminal::Result<Option<u32>> {
        let (exit, master) = {
            let mut session = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let exit = session.close(terminate);
            (exit, session.master.take())
        };
        // The reader remains active to drain ConPTY while its master closes.
        drop(master);
        exit.map(|exit| exit.code)
            .map_err(|error| capstan_terminal::TerminalError::Backend(error.code().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{PtyCommand, PtySessionConfig};
    use crate::{PtyError, ShellInfo, ShellSource};

    #[test]
    fn rejects_zero_terminal_dimensions() {
        let config = PtySessionConfig {
            command: PtyCommand {
                shell: ShellInfo::powershell_51("powershell.exe".into(), ShellSource::UserProvided),
                working_directory: None,
                environment: BTreeMap::new(),
            },
            columns: 0,
            rows: 24,
        };
        assert!(matches!(
            config.validate(),
            Err(PtyError::InvalidSize { .. })
        ));
    }

    #[test]
    fn rejects_empty_shell_path() {
        let config = PtySessionConfig {
            command: PtyCommand {
                shell: ShellInfo {
                    kind: crate::ShellKind::Custom,
                    executable: "".into(),
                    args: Vec::new(),
                    display_name: "custom".to_owned(),
                    source: ShellSource::UserProvided,
                },
                working_directory: None,
                environment: BTreeMap::new(),
            },
            columns: 80,
            rows: 24,
        };
        assert!(matches!(config.validate(), Err(PtyError::ShellNotFound(_))));
    }
}
