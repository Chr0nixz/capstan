use super::*;
use crate::{
    connect_interactive, connect_password, HostPublicKey, PasswordConnection, TrustBroker,
    TrustRepository,
};
use russh::{
    keys::{Algorithm, PrivateKey},
    server::{self, Auth, Response},
    MethodSet,
};
use std::{borrow::Cow, collections::VecDeque, sync::Mutex};
use tokio::net::TcpListener;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    /// `PasswordAuthentication no` with PAM asking for the password, then a code.
    PasswordThenCode,
    /// Only the password method, accepting `secret`.
    PasswordOnly,
    /// Keyboard-interactive requests that never end.
    Endless,
    /// One request whose text carries control and bidirectional characters.
    Hostile,
    /// pam_oath alone: one hidden "One-time password" prompt wanting `123456`.
    Oath,
}

#[derive(Clone)]
struct Server {
    script: Script,
    stage: usize,
    passwords: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Server {
    fn methods(&self) -> MethodSet {
        MethodSet::from(
            &[if self.script == Script::PasswordOnly {
                MethodKind::Password
            } else {
                MethodKind::KeyboardInteractive
            }][..],
        )
    }
    fn reject(&self) -> Auth {
        Auth::Reject {
            proceed_with_methods: Some(self.methods()),
            partial_success: false,
        }
    }
}

fn request(
    name: &'static str,
    instructions: &'static str,
    prompt: &'static str,
    echo: bool,
) -> Auth {
    Auth::Partial {
        name: Cow::Borrowed(name),
        instructions: Cow::Borrowed(instructions),
        prompts: Cow::Owned(vec![(Cow::Borrowed(prompt), echo)]),
    }
}

impl server::Handler for Server {
    type Error = russh::Error;
    async fn auth_none(&mut self, _: &str) -> std::result::Result<Auth, Self::Error> {
        Ok(self.reject())
    }
    async fn auth_password(
        &mut self,
        _: &str,
        password: &str,
    ) -> std::result::Result<Auth, Self::Error> {
        self.passwords.lock().unwrap().push(password.to_owned());
        if self.script == Script::PasswordOnly && password == "secret" {
            Ok(Auth::Accept)
        } else {
            Ok(self.reject())
        }
    }
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> std::result::Result<Auth, Self::Error> {
        if let Some(response) = response {
            let answers = response
                .map(|answer| String::from_utf8_lossy(&answer).into_owned())
                .collect::<Vec<_>>();
            self.answers.lock().unwrap().push(answers.clone());
            let expected = match (self.script, self.stage) {
                (Script::PasswordThenCode, 1) => "secret",
                (Script::PasswordThenCode, 2) | (Script::Oath, 1) => "123456",
                _ => "",
            };
            if matches!(self.script, Script::PasswordThenCode | Script::Oath)
                && answers != [expected]
            {
                return Ok(self.reject());
            }
        }
        self.stage += 1;
        Ok(match (self.script, self.stage) {
            (Script::PasswordThenCode, 1) => request("", "", "Password: ", false),
            (Script::PasswordThenCode, 2) => request("", "", "Verification code: ", false),
            (Script::Endless, _) => request("", "", "Code: ", false),
            (Script::Oath, 1) => request("", "", "One-time password (OATH) for `alice': ", false),
            (Script::Hostile, 1) => request(
                "Login\u{1b}[2J",
                "Line one\nLine\u{202e} two\r",
                "Code\u{7}: ",
                true,
            ),
            _ => Auth::Accept,
        })
    }
}

struct TestServer {
    address: std::net::SocketAddr,
    key: HostPublicKey,
    passwords: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<Vec<String>>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(script: Script) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key = HostPublicKey::from_public_key(host_key.public_key()).unwrap();
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key],
        ..server::Config::default()
    });
    let passwords = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(Vec::new()));
    let handler = Server {
        script,
        stage: 0,
        passwords: passwords.clone(),
        answers: answers.clone(),
    };
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = config.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    TestServer {
        address,
        key,
        passwords,
        answers,
        task,
    }
}

struct Known(HostPublicKey);
#[async_trait]
impl TrustRepository for Known {
    async fn lookup(&self, _: &str, _: u16) -> Result<Option<HostPublicKey>> {
        Ok(Some(self.0.clone()))
    }
    async fn is_current(&self, _: &AttemptBinding) -> bool {
        true
    }
    async fn remember(&self, _: &AttemptBinding, _: &HostPublicKey) -> Result<()> {
        Err(SshError::Protocol)
    }
}

/// Answers prompts from a queue; an empty queue declines.
#[derive(Default)]
struct Scripted {
    answers: Mutex<VecDeque<Vec<&'static str>>>,
    seen: Mutex<Vec<AuthPrompt>>,
    repeat: Option<&'static str>,
}
impl Scripted {
    fn answering(answers: &[&[&'static str]]) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.iter().map(|answer| answer.to_vec()).collect()),
            ..Self::default()
        })
    }
    fn seen(&self) -> Vec<AuthPrompt> {
        self.seen.lock().unwrap().clone()
    }
}
#[async_trait]
impl AuthPrompter for Scripted {
    async fn answer(&self, prompt: AuthPrompt) -> Option<Vec<SecretString>> {
        self.seen.lock().unwrap().push(prompt);
        let answer = match self.repeat {
            Some(answer) => vec![answer],
            None => self.answers.lock().unwrap().pop_front()?,
        };
        Some(
            answer
                .into_iter()
                .map(|value| SecretString::from(value.to_owned()))
                .collect(),
        )
    }
}

fn binding(server: &TestServer) -> AttemptBinding {
    AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: Uuid::new_v4(),
        host_revision: 1,
        vault_epoch: 1,
        target: server.address.ip().to_string(),
        port: server.address.port(),
    }
}

async fn connect(
    server: &TestServer,
    password: Option<&str>,
    prompter: Option<Arc<Scripted>>,
) -> Result<PasswordConnection> {
    let (broker, _events) = TrustBroker::new();
    let attempt = binding(server);
    let _permit = prompter.map(|prompter| {
        broker.allow_prompts(attempt.attempt_id, prompter as Arc<dyn AuthPrompter>)
    });
    let repository: Arc<dyn TrustRepository> = Arc::new(Known(server.key.clone()));
    let address = server.address.ip().to_string();
    let port = server.address.port();
    let cancel = CancellationToken::new();
    match password {
        Some(password) => {
            connect_password(
                address,
                port,
                "alice".into(),
                SecretString::from(password.to_owned()),
                attempt,
                repository,
                broker.clone(),
                cancel,
            )
            .await
        }
        None => {
            connect_interactive(
                address,
                port,
                "alice".into(),
                attempt,
                repository,
                broker.clone(),
                cancel,
            )
            .await
        }
    }
}

#[tokio::test]
async fn a_stored_password_answers_only_the_password_prompt_and_the_user_gives_the_code() {
    let server = start(Script::PasswordThenCode).await;
    let prompter = Scripted::answering(&[&["123456"]]);
    connect(&server, Some("secret"), Some(prompter.clone()))
        .await
        .unwrap();
    assert_eq!(
        *server.answers.lock().unwrap(),
        vec![vec!["secret".to_owned()], vec!["123456".to_owned()]]
    );
    let seen = prompter.seen();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].password);
    assert_eq!(seen[0].username, "alice");
    assert_eq!(
        seen[0].fields,
        vec![PromptField {
            text: "Verification code:".into(),
            echo: false
        }]
    );
}

#[tokio::test]
async fn a_one_time_password_prompt_asks_the_user_instead_of_taking_the_stored_password() {
    let server = start(Script::Oath).await;
    let prompter = Scripted::answering(&[&["123456"]]);
    connect(&server, Some("secret"), Some(prompter.clone()))
        .await
        .unwrap();
    assert_eq!(
        *server.answers.lock().unwrap(),
        vec![vec!["123456".to_owned()]]
    );
    assert_eq!(prompter.seen().len(), 1);
}

#[tokio::test]
async fn without_a_permit_a_code_prompt_fails_instead_of_waiting() {
    let server = start(Script::PasswordThenCode).await;
    let error = connect(&server, Some("secret"), None).await.err();
    assert_eq!(error, Some(SshError::InteractionRequired));
    assert_eq!(
        *server.answers.lock().unwrap(),
        vec![vec!["secret".to_owned()]]
    );
}

#[tokio::test]
async fn interactive_credentials_ask_for_the_password_method_and_keyboard_prompts() {
    let server = start(Script::PasswordOnly).await;
    let prompter = Scripted::answering(&[&["secret"]]);
    connect(&server, None, Some(prompter.clone()))
        .await
        .unwrap();
    assert_eq!(*server.passwords.lock().unwrap(), vec!["secret".to_owned()]);
    let seen = prompter.seen();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].password);
    assert_eq!(seen[0].fields.len(), 1);

    let server = start(Script::PasswordThenCode).await;
    let prompter = Scripted::answering(&[&["secret"], &["123456"]]);
    connect(&server, None, Some(prompter.clone()))
        .await
        .unwrap();
    let seen = prompter.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].fields[0].text, "Password:");
}

#[tokio::test]
async fn a_wrong_stored_password_fails_without_asking_the_user() {
    let server = start(Script::PasswordOnly).await;
    let prompter = Scripted::answering(&[&["secret"]]);
    let error = connect(&server, Some("wrong"), Some(prompter.clone()))
        .await
        .err();
    assert_eq!(error, Some(SshError::Authentication));
    assert!(prompter.seen().is_empty());
    assert_eq!(*server.passwords.lock().unwrap(), vec!["wrong".to_owned()]);
}

#[tokio::test]
async fn declining_a_prompt_cancels_the_attempt() {
    let server = start(Script::PasswordThenCode).await;
    let prompter = Scripted::answering(&[]);
    let error = connect(&server, None, Some(prompter.clone())).await.err();
    assert_eq!(error, Some(SshError::Canceled));
    assert_eq!(prompter.seen().len(), 1);
    assert!(server.answers.lock().unwrap().is_empty());
}

#[tokio::test]
async fn endless_requests_stop_after_a_bounded_number_of_rounds() {
    let server = start(Script::Endless).await;
    let prompter = Arc::new(Scripted {
        repeat: Some("000000"),
        ..Scripted::default()
    });
    let error = connect(&server, None, Some(prompter.clone())).await.err();
    assert_eq!(error, Some(SshError::Protocol));
    assert_eq!(prompter.seen().len(), MAX_ROUNDS);
    assert_eq!(server.answers.lock().unwrap().len(), MAX_ROUNDS);
}

#[tokio::test]
async fn server_text_is_bounded_and_stripped_before_it_is_shown() {
    let server = start(Script::Hostile).await;
    let prompter = Scripted::answering(&[&["2"]]);
    connect(&server, None, Some(prompter.clone()))
        .await
        .unwrap();
    let seen = prompter.seen();
    assert_eq!(seen[0].name, "Login [2J");
    assert_eq!(seen[0].instructions, "Line one\nLine two");
    assert_eq!(
        seen[0].fields,
        vec![PromptField {
            text: "Code :".into(),
            echo: true
        }]
    );
}

#[test]
fn display_text_keeps_only_bounded_plain_text() {
    assert_eq!(display_text("a\u{0}b\tc", 16, false), "a b c");
    assert_eq!(display_text("one\ntwo", 16, false), "one two");
    assert_eq!(display_text("one\ntwo", 16, true), "one\ntwo");
    assert_eq!(display_text("\u{2066}abc\u{2069}", 16, false), "abc");
    assert_eq!(
        display_text(&"é".repeat(400), MAX_PROMPT, false)
            .chars()
            .count(),
        MAX_PROMPT
    );
}

#[test]
fn only_password_prompts_receive_a_stored_password() {
    for text in [
        "Password:",
        "alice@host's password: ",
        "Passwort:",
        "密码：",
    ] {
        assert!(asks_for_password(text), "{text}");
    }
    for text in [
        "Verification code:",
        "Passcode or option (1-3):",
        "OTP:",
        "One-time password (OATH) for `alice': ",
        "Enter your one time password:",
        "Password + token code:",
        "OTP password:",
        "一次性密码：",
        "动态密码：",
    ] {
        assert!(!asks_for_password(text), "{text}");
    }
}

#[test]
fn a_permit_allows_prompts_only_while_it_lives() {
    let (broker, _events) = TrustBroker::new();
    let attempt = Uuid::new_v4();
    let permit = broker.allow_prompts(attempt, Scripted::answering(&[]));
    assert!(broker.prompter(attempt).is_some());
    assert!(broker.prompter(Uuid::new_v4()).is_none());
    drop(permit);
    assert!(broker.prompter(attempt).is_none());
}

#[test]
fn diagnostic_codes_name_the_method_without_details() {
    assert_eq!(
        AuthenticationMethod::Password(SecretString::from("secret".to_owned())).code(),
        "password"
    );
    assert_eq!(AuthenticationMethod::Interactive.code(), "interactive");
}
