//! User authentication: the configured method first, then the password or
//! keyboard-interactive steps the server still asks for.
//!
//! A stored password answers one keyboard-interactive prompt that asks for a
//! password. Every other prompt goes to the [`AuthPrompter`] the attempt was
//! given through [`crate::TrustBroker::allow_prompts`]. Without one, a prompt
//! fails the attempt with [`SshError::InteractionRequired`], so automatic
//! reconnects never wait on input nobody was asked for.
use crate::{
    agent_request, open_agent_client, select_agent_identity, AttemptBinding, AuthenticationMethod,
};
use crate::{Result, SshError};
use async_trait::async_trait;
use russh::{
    client::{self, AuthResult, KeyboardInteractiveAuthResponse},
    keys::{HashAlg, PrivateKeyWithHashAlg},
    MethodKind,
};
use secrecy::{ExposeSecret, SecretString};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Network budget for each authentication request.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a prompt waits for the user. OpenSSH drops a session that has not
/// authenticated within `LoginGraceTime` (120 s by default) anyway.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(180);
/// Methods tried after the configured one, so a server cannot loop forever.
const MAX_STEPS: usize = 4;
/// Keyboard-interactive requests answered in one attempt.
const MAX_ROUNDS: usize = 8;
const MAX_FIELDS: usize = 8;
const MAX_NAME: usize = 128;
const MAX_INSTRUCTIONS: usize = 1024;
const MAX_PROMPT: usize = 256;
const MAX_ANSWER: usize = 8192;

/// One question in an [`AuthPrompt`]. `echo` is the server's hint that the
/// answer is not secret, such as a menu choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptField {
    pub text: String,
    pub echo: bool,
}

/// Input the server asks for during authentication. Server text is limited in
/// length and stripped of control and bidirectional formatting characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPrompt {
    pub attempt_id: Uuid,
    pub host_id: Uuid,
    pub target: String,
    pub port: u16,
    pub username: String,
    /// The SSH password method rather than a keyboard-interactive request,
    /// so the fields carry no server text.
    pub password: bool,
    pub name: String,
    pub instructions: String,
    pub fields: Vec<PromptField>,
}

#[async_trait]
pub trait AuthPrompter: Send + Sync + 'static {
    /// One answer per field, in order, or `None` when the user declines.
    async fn answer(&self, prompt: AuthPrompt) -> Option<Vec<SecretString>>;
}

pub(crate) struct Prompts<'a> {
    pub prompter: Option<Arc<dyn AuthPrompter>>,
    pub attempt: &'a AttemptBinding,
    pub cancel: &'a CancellationToken,
}

/// Returns whether the server accepted the user. Transport loss shows up as
/// [`SshError::Authentication`]; the caller tells the two apart.
pub(crate) async fn authenticate<H: client::Handler>(
    handle: &mut client::Handle<H>,
    username: &str,
    method: AuthenticationMethod,
    prompts: Prompts<'_>,
) -> Result<bool> {
    let code = method.code();
    let mut session = Session {
        handle,
        username,
        prompts,
        password: None,
        rounds: 0,
        prompted: false,
    };
    let result = session.run(method).await;
    capstan_diagnostics::event("ssh_auth")
        .code("method", code)
        .code(
            "outcome",
            match &result {
                Ok(true) => "accepted",
                Ok(false) => "rejected",
                Err(error) => error.code(),
            },
        )
        .number("rounds", session.rounds as u64)
        .flag("prompted", session.prompted)
        .record();
    result
}

struct Session<'a, 'b, H: client::Handler> {
    handle: &'a mut client::Handle<H>,
    username: &'a str,
    prompts: Prompts<'b>,
    /// A stored password not yet given to a keyboard-interactive prompt.
    password: Option<SecretString>,
    rounds: usize,
    prompted: bool,
}

impl<H: client::Handler> Session<'_, '_, H> {
    async fn run(&mut self, method: AuthenticationMethod) -> Result<bool> {
        let interactive = matches!(method, AuthenticationMethod::Interactive);
        let username = self.username.to_owned();
        let (keyed, first) = match method {
            AuthenticationMethod::Password(password) => {
                let result = step(
                    self.handle
                        .authenticate_password(&username, password.expose_secret()),
                )
                .await?;
                self.password = Some(password);
                (false, result)
            }
            AuthenticationMethod::PrivateKey(key) => {
                let hash = self.rsa_hash().await;
                let key = PrivateKeyWithHashAlg::new(key, hash);
                (
                    true,
                    step(self.handle.authenticate_publickey(&username, key)).await?,
                )
            }
            AuthenticationMethod::Agent {
                provider,
                fingerprint,
            } => {
                let mut agent = agent_request(open_agent_client(provider)).await?;
                let identities = agent_request(async {
                    agent
                        .request_identities()
                        .await
                        .map_err(|_| SshError::AgentUnavailable)
                })
                .await?;
                let key = select_agent_identity(&identities, &fingerprint)?;
                let hash = self.rsa_hash().await;
                let result = step(
                    self.handle
                        .authenticate_publickey_with(&username, key, hash, &mut agent),
                )
                .await?;
                (true, result)
            }
            AuthenticationMethod::Interactive => {
                // `none` only asks the server which methods it accepts.
                (false, step(self.handle.authenticate_none(&username)).await?)
            }
        };
        let (mut remaining, partial) = match first {
            AuthResult::Success => return Ok(true),
            AuthResult::Failure {
                remaining_methods,
                partial_success,
            } => (remaining_methods, partial_success),
        };
        let continues = partial
            || interactive
            // A server that does not take passwords may still ask for one
            // through keyboard-interactive.
            || (self.password.is_some()
                && !remaining.contains(&MethodKind::Password)
                && remaining.contains(&MethodKind::KeyboardInteractive))
            // A rejected key falls back to the server's prompts, as OpenSSH
            // does, but only when someone is there to answer them.
            || (keyed && self.prompts.prompter.is_some());
        if !continues {
            return Ok(false);
        }
        for _ in 0..MAX_STEPS {
            let result = if remaining.contains(&MethodKind::KeyboardInteractive) {
                self.keyboard_interactive().await?
            } else if remaining.contains(&MethodKind::Password) {
                self.password_prompt().await?
            } else {
                return Ok(false);
            };
            match result {
                AuthResult::Success => return Ok(true),
                AuthResult::Failure {
                    remaining_methods,
                    partial_success: true,
                } => remaining = remaining_methods,
                AuthResult::Failure { .. } => return Ok(false),
            }
        }
        Ok(false)
    }

    async fn rsa_hash(&mut self) -> Option<HashAlg> {
        self.handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten()
            .or(Some(HashAlg::Sha512))
    }

    async fn keyboard_interactive(&mut self) -> Result<AuthResult> {
        let username = self.username.to_owned();
        let mut reply = step(
            self.handle
                .authenticate_keyboard_interactive_start(username, None::<String>),
        )
        .await?;
        loop {
            match reply {
                KeyboardInteractiveAuthResponse::Success => return Ok(AuthResult::Success),
                KeyboardInteractiveAuthResponse::Failure {
                    remaining_methods,
                    partial_success,
                } => {
                    return Ok(AuthResult::Failure {
                        remaining_methods,
                        partial_success,
                    })
                }
                KeyboardInteractiveAuthResponse::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    self.rounds += 1;
                    if self.rounds > MAX_ROUNDS || prompts.len() > MAX_FIELDS {
                        return Err(SshError::Protocol);
                    }
                    let fields = prompts
                        .into_iter()
                        .map(|prompt| PromptField {
                            text: display_text(&prompt.prompt, MAX_PROMPT, false),
                            echo: prompt.echo,
                        })
                        .collect::<Vec<_>>();
                    let answers = self.answer(name, instructions, fields).await?;
                    reply = step(
                        self.handle
                            .authenticate_keyboard_interactive_respond(answers),
                    )
                    .await?;
                }
            }
        }
    }

    async fn answer(
        &mut self,
        name: String,
        instructions: String,
        fields: Vec<PromptField>,
    ) -> Result<Vec<String>> {
        // Servers send empty requests to show text or to finish a step.
        if fields.is_empty() {
            return Ok(Vec::new());
        }
        if let [field] = fields.as_slice() {
            if !field.echo && asks_for_password(&field.text) {
                if let Some(password) = self.password.take() {
                    return Ok(vec![password.expose_secret().to_owned()]);
                }
            }
        }
        self.ask(AuthPromptText {
            password: false,
            name: display_text(&name, MAX_NAME, false),
            instructions: display_text(&instructions, MAX_INSTRUCTIONS, true),
            fields,
        })
        .await
    }

    async fn password_prompt(&mut self) -> Result<AuthResult> {
        let mut answers = self
            .ask(AuthPromptText {
                password: true,
                name: String::new(),
                instructions: String::new(),
                fields: vec![PromptField {
                    text: String::new(),
                    echo: false,
                }],
            })
            .await?;
        let password = answers.pop().ok_or(SshError::InvalidInput)?;
        let username = self.username.to_owned();
        step(self.handle.authenticate_password(username, password)).await
    }

    async fn ask(&mut self, text: AuthPromptText) -> Result<Vec<String>> {
        let prompter = self
            .prompts
            .prompter
            .clone()
            .ok_or(SshError::InteractionRequired)?;
        let attempt = self.prompts.attempt;
        let count = text.fields.len();
        let prompt = AuthPrompt {
            attempt_id: attempt.attempt_id,
            host_id: attempt.host_id,
            target: attempt.target.clone(),
            port: attempt.port,
            username: self.username.to_owned(),
            password: text.password,
            name: text.name,
            instructions: text.instructions,
            fields: text.fields,
        };
        self.prompted = true;
        let answers = tokio::select! {
            biased;
            () = self.prompts.cancel.cancelled() => return Err(SshError::Canceled),
            answers = tokio::time::timeout(PROMPT_TIMEOUT, prompter.answer(prompt)) => answers,
        };
        let answers = answers
            .map_err(|_| SshError::InteractionRequired)?
            .ok_or(SshError::Canceled)?;
        if answers.len() != count
            || answers
                .iter()
                .any(|answer| answer.expose_secret().len() > MAX_ANSWER)
        {
            return Err(SshError::InvalidInput);
        }
        Ok(answers
            .iter()
            .map(|answer| answer.expose_secret().to_owned())
            .collect())
    }
}

struct AuthPromptText {
    password: bool,
    name: String,
    instructions: String,
    fields: Vec<PromptField>,
}

async fn step<T, E>(request: impl Future<Output = std::result::Result<T, E>>) -> Result<T> {
    match tokio::time::timeout(STEP_TIMEOUT, request).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(SshError::Authentication),
        Err(_) => Err(SshError::Timeout),
    }
}

/// Only a prompt that names a password receives the stored one, so a
/// verification code prompt never sees it. One-time passwords, such as
/// pam_oath's "One-time password (OATH)" prompt, are codes, not the password.
fn asks_for_password(text: &str) -> bool {
    let text = text.to_lowercase();
    ["password", "passwort", "密码"]
        .iter()
        .any(|word| text.contains(word))
        && ![
            "one-time",
            "one time",
            "onetime",
            "otp",
            "oath",
            "token",
            "code",
            "verification",
            "一次性",
            "动态",
            "验证",
        ]
        .iter()
        .any(|word| text.contains(word))
}

/// Invisible formatting and direction marks, overrides and isolates.
fn is_hidden_format(character: char) -> bool {
    matches!(
        character,
        '\u{ad}'
            | '\u{61c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
    )
}

/// Server text shown in a native dialog: bounded, without control characters
/// or invisible formatting that could disguise it. Instructions keep their
/// line breaks, including Unicode line and paragraph separators, but not
/// blank lines or indentation, which could set server text apart to look
/// like the dialog's own wording.
fn display_text(value: &str, limit: usize, lines: bool) -> String {
    let text: String = value
        .chars()
        .filter(|character| !is_hidden_format(*character))
        .map(|character| match character {
            '\n' | '\u{2028}' | '\u{2029}' if lines => '\n',
            '\u{2028}' | '\u{2029}' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .take(limit)
        .collect();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

impl AuthenticationMethod {
    fn code(&self) -> &'static str {
        match self {
            Self::Password(_) => "password",
            Self::PrivateKey(_) => "private_key",
            Self::Agent { .. } => "agent",
            Self::Interactive => "interactive",
        }
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
