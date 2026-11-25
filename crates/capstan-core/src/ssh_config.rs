//! Reads an OpenSSH client configuration into hosts that can be imported.
//!
//! Only `Host` blocks with concrete names become candidates. Settings follow
//! OpenSSH's rule that the first value obtained wins, across every block whose
//! patterns match the name, including `Host *` and lines before the first
//! `Host`. `Match` blocks cannot be evaluated without connecting, so their
//! settings are ignored. `Include` is followed with globbing in the file name
//! only, a bounded depth and a bounded number of files; lines after it belong
//! to the block it sits in again. `IdentityFile` is the exception to "first
//! wins": OpenSSH tries every listed key, and the first one that exists is
//! imported.
//!
//! A single-hop `ProxyJump` becomes a link to the jump host's candidate: the
//! jump name is resolved through the same configuration, as OpenSSH does, and
//! a jump host that is not itself a `Host` entry is added as one.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Largest configuration file read.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 16;
const MAX_DEPTH: usize = 4;
const MAX_CANDIDATES: usize = 512;
const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// File access, so the parser can be tested without a home directory.
pub trait ConfigFiles {
    /// The file's text, or `None` if it is missing, too large or not text.
    fn read(&self, path: &Path) -> Option<String>;
    /// Files in `directory` whose names match `pattern`, sorted by name.
    fn matching(&self, directory: &Path, pattern: &str) -> Vec<PathBuf>;
    fn is_file(&self, path: &Path) -> bool;
}

/// The local file system.
pub struct SystemFiles;
impl ConfigFiles for SystemFiles {
    fn read(&self, path: &Path) -> Option<String> {
        use std::io::Read as _;
        let file = std::fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return None;
        }
        let mut text = String::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_string(&mut text)
            .ok()?;
        (text.len() as u64 <= MAX_FILE_BYTES).then_some(text)
    }
    fn matching(&self, directory: &Path, pattern: &str) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| glob(pattern, name, false))
            })
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        paths
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Note {
    /// `User` was not set, so the local user name is used, as OpenSSH does.
    LocalUser,
    /// No `IdentityFile`; the first default key in `~/.ssh` is used.
    DefaultIdentity,
    /// The `IdentityFile` is missing or uses an unsupported token, so the
    /// host asks at connect time.
    IdentityUnavailable,
    /// `ProxyCommand`, a multi-hop or unusable `ProxyJump`, or a jump host
    /// with one of those; Capstan cannot follow it and connects directly.
    ProxyUnsupported,
    /// Several `IdentityFile` keys exist. OpenSSH tries them in turn; a
    /// Capstan host keeps one, the first that can be read.
    SeveralIdentities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub alias: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    /// Absolute key path, or `None` to answer prompts at connect time.
    pub identity_file: Option<PathBuf>,
    /// Further existing key files the configuration lists, in order. Import
    /// falls back to them when `identity_file` cannot be read.
    pub other_identities: Vec<PathBuf>,
    pub notes: Vec<Note>,
    /// Index of the candidate this host is reached through.
    pub jump: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Skip {
    /// A `Host` name with no usable address, port or user.
    InvalidHost,
    /// Settings inside a `Match` block, which are ignored.
    MatchBlock,
    /// An `Include` that was missing, too deep or past the file limit.
    Include,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skip>,
}

pub struct Environment<'a> {
    /// The user's home directory; `~/.ssh` holds the configuration.
    pub home: &'a Path,
    pub local_user: Option<&'a str>,
    pub files: &'a dyn ConfigFiles,
}

#[derive(Debug, Default, Clone)]
struct Block {
    /// `None` for a `Match` block.
    patterns: Option<Vec<String>>,
    options: Vec<(Keyword, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keyword {
    HostName,
    User,
    Port,
    IdentityFile,
    ProxyJump,
    ProxyCommand,
}

impl Keyword {
    /// `ProxyJump` and `ProxyCommand` compete: whichever comes first wins.
    fn slot(self) -> Self {
        match self {
            Self::ProxyCommand => Self::ProxyJump,
            other => other,
        }
    }
}

struct Reader<'a> {
    environment: &'a Environment<'a>,
    blocks: Vec<Block>,
    files: usize,
    skipped: Vec<Skip>,
}

/// Parses `~/.ssh/config` and the files it includes, or `None` when there is
/// no readable configuration.
#[must_use]
pub fn parse(environment: &Environment<'_>) -> Option<Parsed> {
    let ssh = environment.home.join(".ssh");
    let mut reader = Reader {
        environment,
        // Lines before the first `Host` apply to every host.
        blocks: vec![Block {
            patterns: Some(vec!["*".into()]),
            options: Vec::new(),
        }],
        files: 0,
        skipped: Vec::new(),
    };
    let text = environment.files.read(&ssh.join("config"))?;
    reader.files = 1;
    reader.read(&text, 0);
    let mut parsed = Parsed {
        candidates: Vec::new(),
        skipped: reader.skipped,
    };
    let mut seen = Vec::<String>::new();
    let mut jumps = Vec::new();
    for block in &reader.blocks {
        for alias in block.patterns.iter().flatten() {
            if alias.contains(['*', '?']) || alias.starts_with('!') || seen.contains(alias) {
                continue;
            }
            seen.push(alias.clone());
            if parsed.candidates.len() == MAX_CANDIDATES {
                continue;
            }
            match resolve(environment, &reader.blocks, alias) {
                Some((candidate, jump)) => {
                    parsed.candidates.push(candidate);
                    jumps.push(jump);
                }
                None => parsed.skipped.push(Skip::InvalidHost),
            }
        }
    }
    link_jumps(environment, &reader.blocks, &mut parsed.candidates, jumps);
    Some(parsed)
}

/// A `ProxyJump` hop: `[user@]host[:port]` or the same as an `ssh://` URL.
fn jump_target(text: &str) -> Option<(Option<String>, String, Option<u16>)> {
    let text = text.strip_prefix("ssh://").unwrap_or(text);
    let text = text.strip_suffix('/').unwrap_or(text);
    let (user, rest) = match text.rsplit_once('@') {
        Some((user, rest)) if !user.is_empty() => (Some(user.to_owned()), rest),
        Some(_) => return None,
        None => (None, text),
    };
    let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']')?;
        let port = match after {
            "" => None,
            after => Some(after.strip_prefix(':')?),
        };
        (host, port)
    } else {
        match rest.split_once(':') {
            Some((host, port)) if !port.contains(':') => (host, Some(port)),
            _ => (rest, None),
        }
    };
    let port = match port {
        Some(port) => Some(port.parse::<u16>().ok().filter(|port| *port != 0)?),
        None => None,
    };
    (!host.is_empty()).then(|| (user, host.to_owned(), port))
}

/// Links each candidate to the candidate of its `ProxyJump` host, adding jump
/// hosts that have no `Host` entry of their own. Hops that cannot be followed,
/// loops and chains longer than [`capstan_vault::MAX_JUMP_HOPS`] are noted as
/// unsupported, and so is every host reached through such a hop.
fn link_jumps(
    environment: &Environment<'_>,
    blocks: &[Block],
    candidates: &mut Vec<Candidate>,
    mut jumps: Vec<Option<String>>,
) {
    // Each candidate's own route as its config states it: the `ProxyJump`
    // spelling, and whether a `ProxyCommand` or multi-hop jump was set. A hop
    // reuses a candidate only when it would take the same route.
    let mut routes: Vec<(Option<String>, bool)> = jumps
        .iter()
        .zip(candidates.iter())
        .map(|(jump, candidate)| {
            (
                jump.clone(),
                candidate.notes.contains(&Note::ProxyUnsupported),
            )
        })
        .collect();
    let mut index = 0;
    while index < candidates.len() {
        let Some(spec) = jumps.get_mut(index).and_then(Option::take) else {
            index += 1;
            continue;
        };
        let linked = jump_target(&spec).and_then(|(user, name, port)| {
            if user.is_none() && port.is_none() {
                if let Some(found) = candidates
                    .iter()
                    .position(|candidate| candidate.alias == name)
                {
                    return Some(found);
                }
            }
            let (mut hop, hop_jump) = resolve(environment, blocks, &name)?;
            if let Some(user) = user {
                hop.notes.retain(|note| *note != Note::LocalUser);
                hop.username = user;
            }
            if let Some(port) = port {
                hop.port = port;
            }
            hop.alias = spec.chars().take(256).collect();
            let route = (
                hop_jump.clone(),
                hop.notes.contains(&Note::ProxyUnsupported),
            );
            if let Some(found) = candidates
                .iter()
                .zip(&routes)
                .position(|(candidate, known)| {
                    candidate.address == hop.address
                        && candidate.port == hop.port
                        && candidate.username == hop.username
                        && candidate.identity_file == hop.identity_file
                        && *known == route
                })
            {
                return Some(found);
            }
            if candidates.len() == MAX_CANDIDATES || !valid_user(&hop.username) {
                return None;
            }
            candidates.push(hop);
            jumps.push(hop_jump);
            routes.push(route);
            Some(candidates.len() - 1)
        });
        match linked {
            Some(jump) if jump != index => candidates[index].jump = Some(jump),
            _ => add_note(&mut candidates[index], Note::ProxyUnsupported),
        }
        index += 1;
    }
    // Break loops and over-long chains where they are found.
    for index in 0..candidates.len() {
        let mut seen = vec![index];
        let mut next = candidates[index].jump;
        while let Some(hop) = next {
            if seen.contains(&hop) || seen.len() > capstan_vault::MAX_JUMP_HOPS {
                candidates[index].jump = None;
                add_note(&mut candidates[index], Note::ProxyUnsupported);
                break;
            }
            seen.push(hop);
            next = candidates[hop].jump;
        }
    }
    // A host is only as reachable as every hop on its way.
    loop {
        let mut changed = false;
        for index in 0..candidates.len() {
            let blocked = candidates[index]
                .jump
                .is_some_and(|hop| candidates[hop].notes.contains(&Note::ProxyUnsupported));
            if blocked && !candidates[index].notes.contains(&Note::ProxyUnsupported) {
                candidates[index].notes.push(Note::ProxyUnsupported);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn add_note(candidate: &mut Candidate, note: Note) {
    if !candidate.notes.contains(&note) {
        candidate.notes.push(note);
    }
}

fn valid_user(username: &str) -> bool {
    !username.is_empty()
        && username.chars().count() <= 256
        && !username.chars().any(char::is_control)
}

/// The home directory and user name OpenSSH would use, from the environment.
#[must_use]
pub fn user_environment() -> Option<(PathBuf, Option<String>)> {
    let (home, user) = if cfg!(windows) {
        ("USERPROFILE", "USERNAME")
    } else {
        ("HOME", "USER")
    };
    let home = PathBuf::from(std::env::var_os(home)?);
    home.is_absolute().then(|| (home, std::env::var(user).ok()))
}

impl Reader<'_> {
    fn read(&mut self, text: &str, depth: usize) {
        for line in text.lines() {
            let Some((keyword, arguments)) = split_line(line) else {
                continue;
            };
            match keyword.as_str() {
                "host" => self.blocks.push(Block {
                    patterns: Some(arguments),
                    options: Vec::new(),
                }),
                "match" => self.blocks.push(Block::default()),
                "include" => {
                    let blocks = self.blocks.len();
                    let current = self.blocks.last().map(|block| block.patterns.clone());
                    self.include(&arguments, depth);
                    // OpenSSH returns to the including block afterwards; the
                    // next lines must not land in the included file's last block.
                    if let (true, Some(patterns)) = (self.blocks.len() != blocks, current) {
                        self.blocks.push(Block {
                            patterns,
                            options: Vec::new(),
                        });
                    }
                }
                _ => {
                    let keyword = match keyword.as_str() {
                        "hostname" => Keyword::HostName,
                        "user" => Keyword::User,
                        "port" => Keyword::Port,
                        "identityfile" => Keyword::IdentityFile,
                        "proxyjump" => Keyword::ProxyJump,
                        "proxycommand" => Keyword::ProxyCommand,
                        _ => continue,
                    };
                    let Some(value) = arguments.into_iter().next() else {
                        continue;
                    };
                    if let Some(block) = self.blocks.last_mut() {
                        if block.patterns.is_none() {
                            if !self.skipped.contains(&Skip::MatchBlock) {
                                self.skipped.push(Skip::MatchBlock);
                            }
                        } else {
                            block.options.push((keyword, value));
                        }
                    }
                }
            }
        }
    }

    fn include(&mut self, arguments: &[String], depth: usize) {
        let ssh = self.environment.home.join(".ssh");
        for argument in arguments {
            let path = expand_home(argument, self.environment.home);
            // OpenSSH reads relative user includes from ~/.ssh.
            let path = if path.is_absolute() {
                path
            } else {
                ssh.join(path)
            };
            let (Some(directory), Some(name)) = (
                path.parent(),
                path.file_name().and_then(|name| name.to_str()),
            ) else {
                self.skipped.push(Skip::Include);
                continue;
            };
            let files = if name.contains(['*', '?']) {
                self.environment.files.matching(directory, name)
            } else {
                vec![path.clone()]
            };
            if files.is_empty() {
                self.skipped.push(Skip::Include);
            }
            for file in files {
                if depth + 1 > MAX_DEPTH || self.files == MAX_FILES {
                    self.skipped.push(Skip::Include);
                    continue;
                }
                let Some(text) = self.environment.files.read(&file) else {
                    self.skipped.push(Skip::Include);
                    continue;
                };
                self.files += 1;
                self.read(&text, depth + 1);
            }
        }
    }
}

/// The candidate for `alias` and its `ProxyJump` hop, if it has one.
fn resolve(
    environment: &Environment<'_>,
    blocks: &[Block],
    alias: &str,
) -> Option<(Candidate, Option<String>)> {
    let mut values: Vec<(Keyword, &str)> = Vec::new();
    // Every IdentityFile counts, in order; OpenSSH tries each one.
    let mut identities: Vec<&str> = Vec::new();
    for block in blocks {
        let Some(patterns) = &block.patterns else {
            continue;
        };
        if !matches_host(patterns, alias) {
            continue;
        }
        for (keyword, value) in &block.options {
            if *keyword == Keyword::IdentityFile {
                if !identities.contains(&value.as_str()) {
                    identities.push(value);
                }
            } else if !values
                .iter()
                .any(|(known, _)| known.slot() == keyword.slot())
            {
                values.push((*keyword, value));
            }
        }
    }
    let value = |keyword| {
        values
            .iter()
            .find(|(known, _)| *known == keyword)
            .map(|(_, value)| *value)
    };
    let mut notes = Vec::new();
    let address = expand_tokens(value(Keyword::HostName).unwrap_or(alias), alias, None, None)?;
    capstan_vault::normalize_target(&address).ok()?;
    let port = match value(Keyword::Port) {
        Some(port) => port.parse::<u16>().ok().filter(|port| *port != 0)?,
        None => 22,
    };
    let username = match value(Keyword::User) {
        Some(user) => user.to_owned(),
        None => {
            notes.push(Note::LocalUser);
            environment.local_user?.to_owned()
        }
    };
    if alias.chars().count() > 256 || alias.chars().any(char::is_control) || !valid_user(&username)
    {
        return None;
    }
    if value(Keyword::ProxyCommand).is_some_and(|proxy| !proxy.eq_ignore_ascii_case("none")) {
        notes.push(Note::ProxyUnsupported);
    }
    let jump = value(Keyword::ProxyJump)
        .filter(|jump| !jump.eq_ignore_ascii_case("none"))
        .and_then(|jump| {
            if jump.contains(',') {
                notes.push(Note::ProxyUnsupported);
                None
            } else {
                Some(jump.to_owned())
            }
        });
    let home = environment.home;
    let (identity_file, other_identities) = if identities.is_empty() {
        let file = DEFAULT_IDENTITIES
            .iter()
            .map(|name| home.join(".ssh").join(name))
            .find(|file| environment.files.is_file(file));
        if file.is_some() {
            notes.push(Note::DefaultIdentity);
        }
        (file, Vec::new())
    } else {
        let listed: Vec<&str> = identities
            .into_iter()
            .filter(|file| !file.eq_ignore_ascii_case("none"))
            .collect();
        let mut found: Vec<PathBuf> = Vec::new();
        for file in &listed {
            let file = expand_tokens(file, &address, Some(&username), Some(home))
                .map(|file| expand_home(&file, home))
                .map(|file| {
                    if file.is_absolute() {
                        file
                    } else {
                        home.join(file)
                    }
                })
                .filter(|file| environment.files.is_file(file));
            if let Some(file) = file.filter(|file| !found.contains(file)) {
                found.push(file);
            }
        }
        if found.is_empty() && !listed.is_empty() {
            notes.push(Note::IdentityUnavailable);
        }
        if found.len() > 1 {
            notes.push(Note::SeveralIdentities);
        }
        let mut found = found.into_iter();
        (found.next(), found.collect())
    };
    Some((
        Candidate {
            alias: alias.to_owned(),
            address,
            port,
            username,
            identity_file,
            other_identities,
            notes,
            jump: None,
        },
        jump,
    ))
}

/// A host matches when one pattern matches and no negated pattern does.
fn matches_host(patterns: &[String], host: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob(negated, host, true) {
                return false;
            }
        } else if glob(pattern, host, true) {
            matched = true;
        }
    }
    matched
}

/// `*` and `?` wildcards, as in OpenSSH patterns and include file names.
fn glob(pattern: &str, value: &str, ignore_case: bool) -> bool {
    let normalize = |text: &str| -> Vec<char> {
        if ignore_case {
            text.to_ascii_lowercase().chars().collect()
        } else {
            text.chars().collect()
        }
    };
    let (pattern, value) = (normalize(pattern), normalize(value));
    let (mut p, mut v) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, v));
            p += 1;
        } else if let Some((star_p, star_v)) = star {
            p = star_p + 1;
            v = star_v + 1;
            star = Some((star_p, star_v + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

/// Expands the OpenSSH tokens known before connecting: `%h` the host, `%r`
/// the remote user, `%d` the home directory and `%%` a percent sign. Any
/// other token makes the value unusable.
fn expand_tokens(
    value: &str,
    host: &str,
    user: Option<&str>,
    home: Option<&Path>,
) -> Option<String> {
    let mut expanded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.next()? {
            '%' => expanded.push('%'),
            'h' => expanded.push_str(host),
            'r' => expanded.push_str(user?),
            'd' => expanded.push_str(home?.to_str()?),
            _ => return None,
        }
    }
    Some(expanded)
}

fn expand_home(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        home.to_owned()
    } else if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        // Rebuild with the platform separator so `~/.ssh/key` names the same
        // path, in the same form, as a key chosen in the file picker.
        let mut path = home.to_owned();
        path.extend(Path::new(rest).components());
        path
    } else {
        PathBuf::from(value)
    }
}

/// Keyword in lower case and its arguments. Arguments may be double-quoted;
/// an unquoted `#` starts a comment.
fn split_line(line: &str) -> Option<(String, Vec<String>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|character: char| character.is_whitespace() || character == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_ascii_lowercase();
    let mut rest = line[end..].trim_start();
    if let Some(after) = rest.strip_prefix('=') {
        rest = after.trim_start();
    }
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in rest.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '#' if !quoted && !started => break,
            character if character.is_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            character => {
                current.push(character);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        arguments.push(current);
    }
    Some((keyword, arguments))
}

#[cfg(test)]
#[path = "ssh_config_tests.rs"]
mod tests;
