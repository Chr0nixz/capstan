#![allow(clippy::pedantic)]

#[path = "real_sshd/forwarding.rs"]
mod forwarding;

use async_trait::async_trait;
use capstan_ssh::{
    connect_agent, connect_password, connect_private_key, list_agent_identities, AgentProvider,
    AttemptBinding, ChannelExit, HostPublicKey, PasswordConnection, SshError, SshShell,
    TrustBroker, TrustRepository,
};
use secrecy::SecretString;
use std::{
    env,
    future::Future,
    io::{Read, Write},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::AsyncReadExt, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

type ConnectFuture = Pin<Box<dyn Future<Output = capstan_ssh::Result<PasswordConnection>> + Send>>;

struct TestTarget {
    username: String,
    address: String,
    port: u16,
}

#[derive(Clone)]
enum TestAuthentication {
    Password(SecretString),
    PrivateKey {
        content: Zeroizing<String>,
        passphrase: Option<Zeroizing<String>>,
    },
    Agent {
        provider: AgentProvider,
        fingerprint: String,
        forwarding_public_key: Option<String>,
    },
}

struct MemoryTrustRepository {
    address: String,
    port: u16,
    key: Mutex<Option<HostPublicKey>>,
}

#[async_trait]
impl TrustRepository for MemoryTrustRepository {
    async fn lookup(&self, address: &str, port: u16) -> capstan_ssh::Result<Option<HostPublicKey>> {
        if address != self.address || port != self.port {
            return Err(SshError::InvalidInput);
        }
        self.key
            .lock()
            .map(|key| key.clone())
            .map_err(|_| SshError::Protocol)
    }

    async fn is_current(&self, attempt: &AttemptBinding) -> bool {
        attempt.target == self.address && attempt.port == self.port
    }

    async fn remember(
        &self,
        attempt: &AttemptBinding,
        key: &HostPublicKey,
    ) -> capstan_ssh::Result<()> {
        if !self.is_current(attempt).await {
            return Err(SshError::Canceled);
        }
        *self.key.lock().map_err(|_| SshError::Protocol)? = Some(key.clone());
        Ok(())
    }
}

fn parse_target(value: &str) -> Result<TestTarget, &'static str> {
    let (username, host_port) = value.split_once('@').ok_or("expected user@host:port")?;
    if username.is_empty()
        || username.len() > 256
        || username.chars().any(char::is_whitespace)
        || host_port.is_empty()
        || host_port.contains('@')
    {
        return Err("invalid user@host:port target");
    }

    let (address, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (ipv6, suffix) = bracketed
            .split_once(']')
            .ok_or("bracketed IPv6 target is missing ]")?;
        let port = suffix
            .strip_prefix(':')
            .ok_or("expected :port after bracketed IPv6 target")?;
        (ipv6, port)
    } else {
        let (address, port) = host_port
            .rsplit_once(':')
            .ok_or("expected user@host:port")?;
        if address.contains(':') {
            return Err("IPv6 targets must use brackets");
        }
        (address, port)
    };
    if address.is_empty() || address.len() > 253 || address.chars().any(char::is_whitespace) {
        return Err("invalid SSH address");
    }
    let port = port.parse::<u16>().map_err(|_| "invalid SSH port")?;
    if port == 0 {
        return Err("SSH port must be between 1 and 65535");
    }
    Ok(TestTarget {
        username: username.to_owned(),
        address: address.to_owned(),
        port,
    })
}

fn read_text_file(
    variable: &str,
    maximum_bytes: usize,
    strip_one_line_ending: bool,
) -> Result<Option<Zeroizing<String>>, String> {
    let Some(path) = env::var_os(variable) else {
        return Ok(None);
    };
    let file = std::fs::File::open(path)
        .map_err(|_| format!("could not read file named by {variable}"))?;
    let mut bytes = Vec::with_capacity(maximum_bytes.saturating_add(1));
    file.take(maximum_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("could not read file named by {variable}"))?;
    let bytes = Zeroizing::new(bytes);
    if bytes.len() > maximum_bytes {
        return Err(format!("file named by {variable} exceeds its size limit"));
    }
    let mut value = Zeroizing::new(
        String::from_utf8(bytes.to_vec())
            .map_err(|_| format!("file named by {variable} must contain UTF-8 text"))?,
    );
    if strip_one_line_ending {
        if value.ends_with("\r\n") {
            let length = value.len();
            value.truncate(length - 2);
        } else if value.ends_with('\n') {
            value.pop();
        }
    }
    if value.is_empty() {
        return Err(format!("file named by {variable} is empty"));
    }
    Ok(Some(value))
}

fn selected_agent_provider() -> Result<Option<AgentProvider>, String> {
    let Some(provider) = env::var("CAPSTAN_TEST_SSH_AGENT").ok() else {
        if env::var_os("CAPSTAN_TEST_SSH_AGENT_FINGERPRINT").is_some()
            || env::var_os("CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY").is_some()
        {
            return Err(
                "Agent fingerprint or forwarding key requires CAPSTAN_TEST_SSH_AGENT".into(),
            );
        }
        return Ok(None);
    };
    match provider.as_str() {
        "openssh" => Ok(Some(AgentProvider::OpenSsh)),
        "pageant" => Ok(Some(AgentProvider::Pageant)),
        _ => Err("CAPSTAN_TEST_SSH_AGENT must be openssh or pageant".into()),
    }
}

fn is_valid_remote_public_key_path(value: &str) -> bool {
    value.starts_with('/') && value.len() <= 4096 && !value.chars().any(char::is_control)
}

async fn configured_authentication() -> Result<Vec<TestAuthentication>, String> {
    let mut methods = Vec::new();
    if let Some(password) = read_text_file("CAPSTAN_TEST_SSH_PASSWORD_FILE", 16 * 1024, true)? {
        methods.push(TestAuthentication::Password(SecretString::from(
            password.as_str().to_owned(),
        )));
    }

    let key = read_text_file("CAPSTAN_TEST_SSH_KEY_FILE", 1024 * 1024, false)?;
    let passphrase = read_text_file("CAPSTAN_TEST_SSH_KEY_PASSPHRASE_FILE", 16 * 1024, true)?;
    if let Some(content) = key {
        methods.push(TestAuthentication::PrivateKey {
            content,
            passphrase,
        });
    } else if passphrase.is_some() {
        return Err("key passphrase file was set without CAPSTAN_TEST_SSH_KEY_FILE".into());
    }

    if let Some(provider) = selected_agent_provider()? {
        let identities = list_agent_identities(provider).await.map_err(|error| {
            format!(
                "could not enumerate configured SSH Agent ({})",
                error.code()
            )
        })?;
        if identities.is_empty() {
            return Err("configured SSH Agent has no selectable public-key identity".into());
        }
        let requested = env::var("CAPSTAN_TEST_SSH_AGENT_FINGERPRINT").ok();
        let fingerprint = match requested {
            Some(fingerprint)
                if identities
                    .iter()
                    .any(|identity| identity.fingerprint == fingerprint) =>
            {
                fingerprint
            }
            Some(_) => {
                return Err("configured fingerprint is not present in the selected Agent".into())
            }
            None if identities.len() == 1 => identities[0].fingerprint.clone(),
            None => {
                return Err(
                    "set CAPSTAN_TEST_SSH_AGENT_FINGERPRINT when the Agent has multiple identities"
                        .into(),
                )
            }
        };
        let forwarding_public_key = match env::var("CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY") {
            Ok(value) if value.trim().is_empty() => {
                return Err("CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY is empty".into());
            }
            Ok(value) if is_valid_remote_public_key_path(&value) => Some(value),
            Ok(_) => {
                return Err(
                    "CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY must be an absolute POSIX path without control characters".into(),
                );
            }
            Err(_) => None,
        };
        methods.push(TestAuthentication::Agent {
            provider,
            fingerprint,
            forwarding_public_key,
        });
    }

    if methods.is_empty() {
        return Err(
            "set a password file, private-key file, or CAPSTAN_TEST_SSH_AGENT for authentication"
                .into(),
        );
    }
    Ok(methods)
}

fn connect_future(
    target: &TestTarget,
    authentication: TestAuthentication,
    attempt: AttemptBinding,
    repository: Arc<dyn TrustRepository>,
    broker: Arc<TrustBroker>,
    cancel: CancellationToken,
) -> ConnectFuture {
    let address = target.address.clone();
    let port = target.port;
    let username = target.username.clone();
    match authentication {
        TestAuthentication::Password(password) => Box::pin(connect_password(
            address, port, username, password, attempt, repository, broker, cancel,
        )),
        TestAuthentication::PrivateKey {
            content,
            passphrase,
        } => Box::pin(async move {
            connect_private_key(
                address,
                port,
                username,
                &content,
                passphrase.as_deref().map(String::as_str),
                attempt,
                repository,
                broker,
                cancel,
            )
            .await
        }),
        TestAuthentication::Agent {
            provider,
            fingerprint,
            forwarding_public_key,
        } => {
            let forward_agent = forwarding_public_key.is_some();
            Box::pin(connect_agent(
                address,
                port,
                username,
                provider,
                fingerprint,
                forward_agent,
                attempt,
                repository,
                broker,
                cancel,
            ))
        }
    }
}

async fn connect_and_accept_ephemeral_trust(
    target: &TestTarget,
    authentication: TestAuthentication,
) -> Result<PasswordConnection, String> {
    let repository = Arc::new(MemoryTrustRepository {
        address: target.address.clone(),
        port: target.port,
        key: Mutex::new(None),
    });
    let repository_for_check: Arc<dyn TrustRepository> = repository.clone();
    let repository_for_connect: Arc<dyn TrustRepository> = repository.clone();
    let (broker, mut events) = TrustBroker::new();
    let attempt = AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        host_revision: 1,
        vault_epoch: 1,
        target: target.address.clone(),
        port: target.port,
    };
    let future = connect_future(
        target,
        authentication,
        attempt,
        repository_for_connect,
        broker.clone(),
        CancellationToken::new(),
    );
    let mut connection_task: JoinHandle<capstan_ssh::Result<PasswordConnection>> =
        tokio::spawn(future);
    let challenge = tokio::select! {
        result = &mut connection_task => {
            return match result {
                Ok(Ok(_)) => Err("connection completed without the expected first-trust challenge".into()),
                Ok(Err(error)) => Err(format!("SSH connection failed before trust ({})", error.code())),
                Err(_) => Err("SSH connection task failed before trust".into()),
            };
        }
        event = timeout(Duration::from_secs(20), events.recv()) => match event {
            Ok(Some(challenge)) => challenge,
            Ok(None) => {
                connection_task.abort();
                return Err("SSH trust event channel closed".into());
            }
            Err(_) => {
                connection_task.abort();
                return Err("timed out waiting for the SSH host-key challenge".into());
            }
        }
    };
    if challenge.target != target.address
        || challenge.port != target.port
        || challenge.key_changed
        || !challenge.fingerprint.starts_with("SHA256:")
    {
        connection_task.abort();
        return Err("received an unexpected SSH host-key challenge".into());
    }
    if let Err(error) = broker.respond(challenge.challenge_id, true) {
        connection_task.abort();
        return Err(format!(
            "could not accept ephemeral test trust ({})",
            error.code()
        ));
    }
    let connection = match timeout(Duration::from_secs(45), &mut connection_task).await {
        Ok(Ok(Ok(connection))) => connection,
        Ok(Ok(Err(error))) => return Err(format!("SSH authentication failed ({})", error.code())),
        Ok(Err(_)) => return Err("SSH connection task failed after trust".into()),
        Err(_) => {
            connection_task.abort();
            return Err("timed out waiting for SSH authentication".into());
        }
    };
    let stored = match repository_for_check
        .lookup(&target.address, target.port)
        .await
    {
        Ok(stored) => stored,
        Err(error) => {
            connection.disconnect().await;
            return Err(format!("ephemeral trust lookup failed ({})", error.code()));
        }
    };
    if stored.is_none_or(|key| key.fingerprint != challenge.fingerprint) {
        connection.disconnect().await;
        return Err("accepted host key was not saved before authentication".into());
    }
    if let Some(path) = env::var_os("CAPSTAN_TEST_SSH_TRUST_FILE") {
        let key = repository_for_check
            .lookup(&target.address, target.port)
            .await
            .map_err(|_| "could not retrieve ephemeral host trust".to_owned())?
            .ok_or_else(|| "ephemeral host trust is missing".to_owned())?;
        let json = serde_json::json!({
            "algorithm": key.algorithm,
            "fingerprint": key.fingerprint,
            "public_key": key.public_key,
        });
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| file.write_all(json.to_string().as_bytes()))
            .map_err(|_| "could not save new ephemeral test trust file".to_owned())?;
    }
    Ok(connection)
}

async fn exercise_shell(shell: &mut SshShell, command: &str) -> Result<Vec<u8>, String> {
    shell
        .control
        .write(format!("{command}\n").into_bytes())
        .await
        .map_err(|error| format!("could not write to SSH shell ({})", error.code()))?;
    let mut output = Vec::new();
    match timeout(
        Duration::from_secs(20),
        shell.reader.read_to_end(&mut output),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return Err("could not read SSH shell output".into()),
        Err(_) => return Err("timed out waiting for SSH shell exit".into()),
    }
    wait_for_successful_exit(&mut shell.exit).await?;
    Ok(output)
}

async fn wait_for_successful_exit(
    exit: &mut tokio::sync::watch::Receiver<ChannelExit>,
) -> Result<(), String> {
    let final_state = timeout(
        Duration::from_secs(20),
        exit.wait_for(|state| matches!(state, ChannelExit::Ended(_) | ChannelExit::Failed)),
    )
    .await
    .map_err(|_| "timed out waiting for SSH channel close".to_owned())?
    .map_err(|_| "SSH channel exit notification closed".to_owned())?;
    if *final_state != ChannelExit::Ended(Some(0)) {
        return Err("SSH shell did not report exit status 0".into());
    }
    Ok(())
}

fn quote_shell_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn print_marker(marker: &str) -> String {
    let escaped = marker
        .as_bytes()
        .iter()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect::<String>();
    format!("printf '\\n%b\\n' '{escaped}'")
}

fn contains_output_line(output: &[u8], expected: &str) -> bool {
    String::from_utf8_lossy(output)
        .lines()
        .any(|line| line.trim_end_matches('\r') == expected)
}

async fn exercise_connection(
    connection: &PasswordConnection,
    forwarding_public_key: Option<&str>,
) -> Result<(), String> {
    let mut first = connection
        .open_shell(80, 24)
        .await
        .map_err(|error| format!("could not open first SSH shell ({})", error.code()))?;
    let mut second = connection
        .open_shell(80, 24)
        .await
        .map_err(|error| format!("could not open second SSH shell ({})", error.code()))?;
    if first.connection_id() != second.connection_id() {
        return Err("two SSH shells did not share their transport connection".into());
    }
    first
        .control
        .resize(100, 32)
        .await
        .map_err(|error| format!("PTY resize was rejected ({})", error.code()))?;
    let marker = format!("CAPSTAN_M1_{}_世界_🧪", Uuid::new_v4().simple());
    let first_command = format!(
        "unset HISTFILE; {}; stty size; exit 0",
        print_marker(&marker)
    );
    let first_output = exercise_shell(&mut first, &first_command).await?;
    if !contains_output_line(&first_output, &marker) {
        return Err("SSH shell did not return the Unicode marker".into());
    }
    if !contains_output_line(&first_output, "32 100") {
        return Err("PTY resize did not reach the remote shell".into());
    }

    let second_marker = format!("CAPSTAN_M1_SECOND_{}", Uuid::new_v4().simple());
    let failure_marker = format!("CAPSTAN_M1_FAILURE_{}", Uuid::new_v4().simple());
    let mut second_command = format!("unset HISTFILE; {}; exit 0", print_marker(&second_marker));
    if let Some(public_key_path) = forwarding_public_key {
        let quoted_path = quote_shell_argument(public_key_path);
        second_command = format!(
            "unset HISTFILE; if ssh-add -T -- {quoted_path} >/dev/null 2>&1; then {}; else {}; fi; exit 0",
            print_marker(&second_marker), print_marker(&failure_marker)
        );
    }
    let second_output = exercise_shell(&mut second, &second_command).await?;
    if !contains_output_line(&second_output, &second_marker)
        || contains_output_line(&second_output, &failure_marker)
    {
        return Err(if forwarding_public_key.is_some() {
            "remote Agent forwarding signature check failed".into()
        } else {
            "second SSH shell did not return its independent marker".into()
        });
    }
    Ok(())
}

async fn run_configured_method(
    target: &TestTarget,
    authentication: TestAuthentication,
) -> Result<(), String> {
    if env::var("CAPSTAN_TEST_SSH_EXPECT_CHANGED").as_deref() == Ok("1") {
        return verify_changed_host_key(target, authentication).await;
    }
    if let TestAuthentication::Agent {
        provider,
        fingerprint,
        forwarding_public_key: Some(public_key_path),
    } = authentication
    {
        for enabled in [false, true, false] {
            let method = TestAuthentication::Agent {
                provider,
                fingerprint: fingerprint.clone(),
                forwarding_public_key: enabled.then(|| public_key_path.clone()),
            };
            let connection = connect_and_accept_ephemeral_trust(target, method).await?;
            let result = if enabled {
                exercise_connection(&connection, Some(&public_key_path)).await
            } else {
                exercise_disabled_agent_forwarding(&connection, &public_key_path).await
            };
            if result.is_ok() && enabled {
                forwarding::exercise(target, connection).await?;
            } else {
                connection.disconnect().await;
                result?;
            }
        }
        return Ok(());
    }
    let forwarding_public_key = match &authentication {
        TestAuthentication::Agent {
            forwarding_public_key,
            ..
        } => forwarding_public_key.clone(),
        _ => None,
    };
    let connection = connect_and_accept_ephemeral_trust(target, authentication).await?;
    let result = exercise_connection(&connection, forwarding_public_key.as_deref()).await;
    if result.is_ok() {
        forwarding::exercise(target, connection).await
    } else {
        connection.disconnect().await;
        result
    }
}

async fn verify_changed_host_key(
    target: &TestTarget,
    authentication: TestAuthentication,
) -> Result<(), String> {
    let text = read_text_file("CAPSTAN_TEST_SSH_TRUST_FILE", 16 * 1024, false)?
        .ok_or_else(|| "changed-key test requires an ephemeral trust file".to_owned())?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "ephemeral trust file is invalid".to_owned())?;
    let field = |name| {
        json.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "ephemeral trust file is invalid".to_owned())
    };
    let old_key = HostPublicKey {
        algorithm: field("algorithm")?,
        fingerprint: field("fingerprint")?,
        public_key: field("public_key")?,
    };
    let repository = Arc::new(MemoryTrustRepository {
        address: target.address.clone(),
        port: target.port,
        key: Mutex::new(Some(old_key.clone())),
    });
    let (broker, mut events) = TrustBroker::new();
    let attempt = AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        host_revision: 1,
        vault_epoch: 1,
        target: target.address.clone(),
        port: target.port,
    };
    let connection = timeout(
        Duration::from_secs(30),
        connect_future(
            target,
            authentication,
            attempt,
            repository.clone(),
            broker,
            CancellationToken::new(),
        ),
    )
    .await
    .map_err(|_| "changed-key connection did not terminate".to_owned())?;
    match connection {
        Err(SshError::HostKeyChanged) => {}
        Ok(connection) => {
            connection.disconnect().await;
            return Err("changed server key was accepted".into());
        }
        Err(error) => return Err(format!("unexpected changed-key failure ({})", error.code())),
    }
    let notice = timeout(Duration::from_secs(2), events.recv())
        .await
        .map_err(|_| "changed-key notice was not emitted".to_owned())?
        .ok_or_else(|| "changed-key event channel closed".to_owned())?;
    if !notice.key_changed
        || notice.known_fingerprint.as_deref() != Some(old_key.fingerprint.as_str())
        || notice.known_algorithm.as_deref() != Some(old_key.algorithm.as_str())
        || notice.fingerprint == old_key.fingerprint
        || repository
            .lookup(&target.address, target.port)
            .await
            .ok()
            .flatten()
            != Some(old_key)
    {
        return Err("changed-key comparison or unchanged trust check failed".into());
    }
    Ok(())
}

async fn exercise_disabled_agent_forwarding(
    connection: &PasswordConnection,
    public_key_path: &str,
) -> Result<(), String> {
    let mut shell = connection
        .open_shell(80, 24)
        .await
        .map_err(|error| format!("could not open non-forwarded SSH shell ({})", error.code()))?;
    let marker = format!("CAPSTAN_AGENT_DISABLED_{}", Uuid::new_v4().simple());
    let command = format!(
        "unset HISTFILE; if [ -z \"${{SSH_AUTH_SOCK:-}}\" ] && ! ssh-add -l >/dev/null 2>&1 && ! ssh-add -T -- {} >/dev/null 2>&1; then {}; fi; exit 0",
        quote_shell_argument(public_key_path), print_marker(&marker)
    );
    let output = exercise_shell(&mut shell, &command).await?;
    if !contains_output_line(&output, &marker) {
        return Err("disabled Agent forwarding exposed a remote Agent".into());
    }
    Ok(())
}

#[tokio::test]
async fn configured_openssh_target_supports_authentication_pty_unicode_and_multiple_shells() {
    let Some(target_value) = env::var("CAPSTAN_TEST_SSHD")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        println!("SKIP: CAPSTAN_TEST_SSHD is not set; no external SSH server was contacted");
        return;
    };
    let target = parse_target(&target_value)
        .unwrap_or_else(|error| panic!("invalid CAPSTAN_TEST_SSHD: {error}"));
    let authentication = configured_authentication()
        .await
        .unwrap_or_else(|error| panic!("invalid real-sshd test configuration: {error}"));
    println!(
        "Running real-sshd compatibility checks for {} configured authentication method(s)",
        authentication.len()
    );
    for method in authentication {
        run_configured_method(&target, method)
            .await
            .unwrap_or_else(|error| panic!("real-sshd compatibility check failed: {error}"));
    }
}

#[test]
fn test_target_parser_handles_dns_ipv4_ipv6_and_rejects_ambiguous_targets() {
    assert_eq!(
        parse_target("tester@127.0.0.1:2222").unwrap().address,
        "127.0.0.1"
    );
    assert_eq!(parse_target("tester@example.test:22").unwrap().port, 22);
    assert_eq!(parse_target("tester@[::1]:2222").unwrap().address, "::1");
    assert!(parse_target("tester@::1:2222").is_err());
    assert!(parse_target("tester@example.test:0").is_err());
    assert!(parse_target("tester@example.test").is_err());
}

#[test]
fn forwarding_path_is_shell_quoted_without_interpreting_quotes() {
    assert_eq!(quote_shell_argument("~/key.pub"), "'~/key.pub'");
    assert_eq!(quote_shell_argument("/tmp/it's.pub"), "'/tmp/it'\\''s.pub'");
    assert!(is_valid_remote_public_key_path("/home/tester/key.pub"));
    assert!(!is_valid_remote_public_key_path("~/key.pub"));
    assert!(!is_valid_remote_public_key_path("-key.pub"));
    assert!(!is_valid_remote_public_key_path("/tmp/key.pub\nexit"));
}

#[test]
fn shell_echo_and_partial_markers_do_not_count_as_probe_output() {
    let marker = "CAPSTAN_TEST_世界_🧪";
    let command = print_marker(marker);
    assert!(!command.contains(marker));
    assert!(!contains_output_line(command.as_bytes(), marker));
    assert!(!contains_output_line(
        format!("prefix {marker}\r\n{marker} suffix\r\n").as_bytes(),
        marker
    ));
    assert!(contains_output_line(
        format!("{command}\r\n{marker}\r\n").as_bytes(),
        marker
    ));
}

#[tokio::test]
async fn eof_and_exit_status_do_not_complete_the_probe_before_channel_close() {
    let (sender, mut receiver) = tokio::sync::watch::channel(ChannelExit::Open);
    let probe = tokio::spawn(async move { wait_for_successful_exit(&mut receiver).await });
    tokio::task::yield_now().await;
    assert!(!probe.is_finished());
    sender.send_replace(ChannelExit::Status(0));
    tokio::task::yield_now().await;
    assert!(!probe.is_finished());
    sender.send_replace(ChannelExit::Ended(Some(0)));
    probe.await.unwrap().unwrap();

    let (_sender, mut receiver) = tokio::sync::watch::channel(ChannelExit::Ended(Some(7)));
    assert!(wait_for_successful_exit(&mut receiver).await.is_err());
}
