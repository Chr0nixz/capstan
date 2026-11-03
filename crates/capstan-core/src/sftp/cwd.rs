//! Untrusted OSC 7 hints never grant filesystem or connection authority.
use super::{RemotePath, SftpError, MAX_OSC_BYTES};
use std::{collections::HashSet, net::IpAddr};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CwdSnapshot {
    pub sequence: super::ExactU64,
    pub path: Option<RemotePath>,
    pub aliases: Vec<String>,
}

/// One terminal's untrusted hints. This state never owns a browsing cursor.
pub(super) struct CwdState {
    target: String,
    aliases: Vec<String>,
    observed: super::ExactU64,
    last_uri: Option<String>,
    latest: Option<(super::ExactU64, String, RemotePath)>,
}
impl CwdState {
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            aliases: Vec::new(),
            observed: super::ExactU64(0),
            last_uri: None,
            latest: None,
        }
    }

    fn snapshot(&self) -> CwdSnapshot {
        CwdSnapshot {
            sequence: self
                .latest
                .as_ref()
                .map_or(super::ExactU64(0), |value| value.0),
            path: self.latest.as_ref().map(|value| value.2.clone()),
            aliases: self.aliases.clone(),
        }
    }

    pub fn update(
        &mut self,
        sequence: super::ExactU64,
        uri: &str,
    ) -> Result<CwdSnapshot, SftpError> {
        if uri.len() > MAX_OSC_BYTES {
            return Err(SftpError::ResourceLimit);
        }
        // A duplicate/late event cannot replace any newer observed hint. A
        // reattached view may safely obtain the existing snapshot this way.
        if sequence <= self.observed {
            return Ok(self.snapshot());
        }
        self.observed = sequence;
        self.last_uri = Some(uri.to_owned());
        if let Ok(path) = CwdAuthority::new(&self.target, &self.aliases)
            .and_then(|authority| authority.parse(uri))
        {
            self.latest = Some((sequence, uri.to_owned(), path));
        }
        Ok(self.snapshot())
    }

    pub fn configure(&mut self, aliases: Option<Vec<String>>) -> Result<CwdSnapshot, SftpError> {
        if let Some(aliases) = aliases {
            let authority = CwdAuthority::new(&self.target, &aliases)?;
            self.aliases = aliases;
            if let Some((uri, path)) = self
                .last_uri
                .as_ref()
                .and_then(|uri| authority.parse(uri).ok().map(|path| (uri.clone(), path)))
            {
                self.latest = Some((self.observed, uri, path));
            } else if self
                .latest
                .as_ref()
                .is_some_and(|value| authority.parse(&value.1).is_err())
            {
                self.latest = None;
            }
        }
        Ok(self.snapshot())
    }
}

/// Constructed only from the authenticated target and aliases explicitly
/// bound by the user to this terminal. Never infer aliases using DNS or OSC.
pub struct CwdAuthority {
    names: HashSet<String>,
}
impl CwdAuthority {
    pub fn new(target: &str, aliases: &[String]) -> Result<Self, SftpError> {
        if aliases.len() > super::MAX_CWD_ALIASES {
            return Err(SftpError::ResourceLimit);
        }
        let mut names = HashSet::new();
        names.insert(normalize_host(target)?);
        for alias in aliases {
            names.insert(normalize_host(alias)?);
        }
        Ok(Self { names })
    }

    /// Decode exactly once, then leave POSIX path resolution to SFTP.
    pub fn parse(&self, uri: &str) -> Result<RemotePath, SftpError> {
        if uri.len() > MAX_OSC_BYTES || uri.chars().any(char::is_control) {
            return Err(SftpError::PathInvalid);
        }
        let rest = uri.strip_prefix("file://").ok_or(SftpError::PathInvalid)?;
        let split = rest.find('/').ok_or(SftpError::PathInvalid)?;
        let authority = &rest[..split];
        if authority.contains(':') && !authority.starts_with('[') {
            return Err(SftpError::PathInvalid);
        }
        let host = normalize_host(authority)?;
        if !self.names.contains(&host) {
            return Err(SftpError::ScopeExpired);
        }
        let path = &rest[split..];
        if path.contains(['?', '#']) {
            return Err(SftpError::PathInvalid);
        }
        let mut decoded = Vec::with_capacity(path.len());
        let mut bytes = path.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let high = bytes.next().and_then(hex).ok_or(SftpError::PathInvalid)?;
                let low = bytes.next().and_then(hex).ok_or(SftpError::PathInvalid)?;
                decoded.push(high * 16 + low);
            } else {
                decoded.push(byte);
            }
        }
        let path = std::str::from_utf8(&decoded).map_err(|_| SftpError::PathInvalid)?;
        RemotePath::parse(path)
    }
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_host(value: &str) -> Result<String, SftpError> {
    // A URI IPv6 literal is bracketed. The trusted target may be unbracketed.
    if value.starts_with('[') && value.ends_with(']') {
        let ip = value[1..value.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .map_err(|_| SftpError::PathInvalid)?;
        return Ok(ip.to_string());
    }
    let host = value;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty()
        || host.len() > 253
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(SftpError::PathInvalid);
    }
    Ok(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn late_invalid_and_cross_host_hints_preserve_latest_valid_state() {
        let mut state = CwdState::new("192.0.2.1");
        let valid = state
            .update(super::super::ExactU64(2), "file://192.0.2.1/a%20%E4%B8%AD")
            .unwrap();
        assert_eq!(valid.path.unwrap().as_str(), "/a 中");
        for (sequence, uri) in [
            (1, "file://192.0.2.1/late"),
            (3, "file://other/wrong"),
            (4, "file://192.0.2.1/%00"),
        ] {
            let snapshot = state.update(super::super::ExactU64(sequence), uri).unwrap();
            assert_eq!(snapshot.sequence.0, 2);
            assert_eq!(snapshot.path.unwrap().as_str(), "/a 中");
        }
        assert_eq!(
            state.update(super::super::ExactU64(5), &"x".repeat(MAX_OSC_BYTES + 1)),
            Err(SftpError::ResourceLimit)
        );
    }

    #[test]
    fn explicit_alias_change_can_revalidate_latest_hint_and_revoke_old_alias() {
        let mut state = CwdState::new("192.0.2.1");
        assert!(state
            .update(super::super::ExactU64(1), "file://shell-host/dir")
            .unwrap()
            .path
            .is_none());
        let accepted = state.configure(Some(vec!["shell-host".into()])).unwrap();
        assert_eq!(accepted.path.unwrap().as_str(), "/dir");
        assert_eq!(accepted.sequence.0, 1);
        assert_eq!(
            state.configure(Some(vec!["user@host".into()])),
            Err(SftpError::PathInvalid)
        );
        assert_eq!(state.configure(None).unwrap().aliases, ["shell-host"]);
        assert!(state.configure(Some(vec![])).unwrap().path.is_none());
        assert!(state
            .update(super::super::ExactU64(1), "file://192.0.2.1/replayed")
            .unwrap()
            .path
            .is_none());
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Case {
        target: String,
        aliases: Vec<String>,
        uri: String,
        path: Option<String>,
        error: Option<SftpError>,
    }

    #[test]
    fn shared_untrusted_uri_fixture_checks_host_and_decodes_only_once() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../../../docs/agent/fixtures/M2-cwd.json"))
                .unwrap();
        for case in cases {
            let authority = CwdAuthority::new(&case.target, &case.aliases).unwrap();
            let actual = authority.parse(&case.uri);
            match case.path {
                Some(path) => assert_eq!(actual.unwrap().as_str(), path, "{}", case.uri),
                None => assert_eq!(actual.err(), case.error, "{}", case.uri),
            }
        }
    }

    #[test]
    fn input_and_decoded_path_have_independent_byte_limits() {
        let authority = CwdAuthority::new("host", &[]).unwrap();
        for uri in [
            format!("file://host/{}", "x".repeat(MAX_OSC_BYTES)),
            format!(
                "file://host/{}",
                "x".repeat(super::super::MAX_REMOTE_PATH_BYTES)
            ),
            format!("file://host/{}", "%41".repeat(MAX_OSC_BYTES / 3)),
        ] {
            assert_eq!(authority.parse(&uri), Err(SftpError::PathInvalid));
        }
        let encoded = format!("file://host/{}", "%41".repeat(2000));
        assert_eq!(authority.parse(&encoded).unwrap().as_str().len(), 2001);
    }

    #[test]
    fn invalid_aliases_cannot_expand_the_scope_or_authorize_implicit_localhost() {
        for target in [
            "",
            "user@host",
            "host:22",
            "host/path",
            "host%2eother",
            "-host",
            "a..b",
        ] {
            assert!(CwdAuthority::new(target, &[]).is_err(), "{target}");
        }
        let too_many = (0..9).map(|_| "alias".to_owned()).collect::<Vec<_>>();
        assert!(CwdAuthority::new("host", &too_many).is_err());
        let authority = CwdAuthority::new("192.0.2.1", &[]).unwrap();
        assert_eq!(
            authority.parse("file://localhost/a"),
            Err(SftpError::ScopeExpired)
        );
        assert_eq!(
            authority.parse("file://127.0.0.1/a"),
            Err(SftpError::ScopeExpired)
        );
    }
}
