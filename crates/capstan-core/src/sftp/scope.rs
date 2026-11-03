use super::{
    DirectoryEntry, DirectoryPage, ExactU64, Metadata, PageInput, RemoteEntryId, RemotePath,
    SftpError, SftpSessionId, DIRECTORY_PAGE_ENTRIES, MAX_BROWSE_SCOPES, MAX_ENTRIES_PER_SCOPE,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

const SCOPE_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Constructed by Rust's terminal registry, never deserialized from a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub window: String,
    pub terminal: Uuid,
    pub connection: Uuid,
    pub generation: u64,
    pub host: Uuid,
}

#[derive(Debug)]
pub struct BrowseScope {
    binding: Binding,
    expires: Instant,
    revision: u64,
    next_page: u32,
    finished: bool,
    path: Option<RemotePath>,
    entries: HashMap<RemoteEntryId, (RemotePath, Metadata)>,
}

/// This bounded registry is only an ownership primitive. It does not look up
/// arbitrary connections by an ID supplied by the WebView.
#[derive(Debug, Default)]
pub struct ScopeRegistry {
    scopes: HashMap<SftpSessionId, BrowseScope>,
}

impl ScopeRegistry {
    pub fn attach(&mut self, binding: Binding, now: Instant) -> Result<SftpSessionId, SftpError> {
        self.scopes.retain(|_, scope| scope.expires > now);
        if self.scopes.len() >= MAX_BROWSE_SCOPES {
            return Err(SftpError::ResourceLimit);
        }
        let id = SftpSessionId::new();
        self.scopes.insert(
            id,
            BrowseScope {
                binding,
                expires: now + SCOPE_LIFETIME,
                revision: 0,
                next_page: 0,
                finished: true,
                path: None,
                entries: HashMap::new(),
            },
        );
        Ok(id)
    }

    fn scope(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        now: Instant,
    ) -> Result<&mut BrowseScope, SftpError> {
        let scope = self.scopes.get_mut(&id).ok_or(SftpError::ScopeExpired)?;
        if scope.expires <= now || &scope.binding != binding {
            return Err(SftpError::ScopeExpired);
        }
        scope.expires = now + SCOPE_LIFETIME;
        Ok(scope)
    }

    pub fn begin_directory(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        path: RemotePath,
        now: Instant,
    ) -> Result<ExactU64, SftpError> {
        let scope = self.scope(id, binding, now)?;
        scope.revision = scope
            .revision
            .checked_add(1)
            .ok_or(SftpError::ResourceLimit)?;
        scope.path = Some(path);
        scope.next_page = 0;
        scope.finished = false;
        scope.entries.clear();
        Ok(ExactU64(scope.revision))
    }

    /// Commit a bounded protocol page only if it still belongs to this view.
    /// Invalid pages do not partially register entries or authorize writes.
    pub fn accept_page(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        input: PageInput,
        now: Instant,
    ) -> Result<DirectoryPage, SftpError> {
        let scope = self.scope(id, binding, now)?;
        Self::commit_page(scope, id, input)
    }

    /// Navigation is transactional: preserve the displayed directory and its
    /// capabilities unless the complete first page can be registered.
    pub fn replace_directory(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        path: RemotePath,
        mut input: PageInput,
        now: Instant,
    ) -> Result<DirectoryPage, SftpError> {
        let scope = self.scope(id, binding, now)?;
        let revision = scope
            .revision
            .checked_add(1)
            .ok_or(SftpError::ResourceLimit)?;
        let mut candidate = BrowseScope {
            binding: binding.clone(),
            expires: scope.expires,
            revision,
            next_page: 0,
            finished: false,
            path: Some(path),
            entries: HashMap::new(),
        };
        input.request_revision = ExactU64(revision);
        let page = Self::commit_page(&mut candidate, id, input)?;
        *scope = candidate;
        Ok(page)
    }

    pub fn authorize(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        now: Instant,
    ) -> Result<(), SftpError> {
        self.scope(id, binding, now).map(|_| ())
    }

    pub fn require_directory(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        path: &RemotePath,
        now: Instant,
    ) -> Result<(), SftpError> {
        let scope = self.scope(id, binding, now)?;
        if scope.next_page == 0 || scope.path.as_ref() != Some(path) {
            return Err(SftpError::ScopeExpired);
        }
        Ok(())
    }

    pub fn directory_path(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        now: Instant,
    ) -> Result<RemotePath, SftpError> {
        self.scope(id, binding, now)?
            .path
            .clone()
            .ok_or(SftpError::ScopeExpired)
    }

    fn commit_page(
        scope: &mut BrowseScope,
        id: SftpSessionId,
        input: PageInput,
    ) -> Result<DirectoryPage, SftpError> {
        if scope.revision != input.request_revision.0
            || scope.next_page != input.page_sequence
            || scope.finished
        {
            return Err(SftpError::ScopeExpired);
        }
        if input.entries.len() > DIRECTORY_PAGE_ENTRIES
            || scope.entries.len() + input.entries.len() > MAX_ENTRIES_PER_SCOPE
        {
            return Err(SftpError::ResourceLimit);
        }
        let next_page = scope
            .next_page
            .checked_add(1)
            .ok_or(SftpError::ResourceLimit)?;
        let path = scope.path.clone().ok_or(SftpError::ScopeExpired)?;
        let mut accepted = Vec::with_capacity(input.entries.len());
        let mut paths = HashSet::new();
        for (name, metadata) in input.entries {
            let full_path = path.join(&name)?;
            if !paths.insert(full_path.clone())
                || scope.entries.values().any(|(path, _)| path == &full_path)
            {
                return Err(SftpError::TargetConflict);
            }
            let entry_id = RemoteEntryId::new();
            accepted.push((
                full_path,
                DirectoryEntry {
                    id: entry_id,
                    name: name.as_str().to_owned(),
                    metadata,
                },
            ));
        }
        for (full_path, entry) in &accepted {
            scope
                .entries
                .insert(entry.id, (full_path.clone(), entry.metadata.clone()));
        }
        scope.next_page = next_page;
        scope.finished = !input.has_more;
        Ok(DirectoryPage {
            session_id: id,
            request_revision: input.request_revision,
            page_sequence: input.page_sequence,
            path,
            entries: accepted.into_iter().map(|(_, entry)| entry).collect(),
            has_more: input.has_more,
            omitted_entries: input.omitted_entries,
        })
    }

    pub fn set_directory_path(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        revision: ExactU64,
        path: RemotePath,
        now: Instant,
    ) -> Result<(), SftpError> {
        let scope = self.scope(id, binding, now)?;
        if scope.revision != revision.0 || scope.next_page != 0 || scope.finished {
            return Err(SftpError::ScopeExpired);
        }
        scope.path = Some(path);
        Ok(())
    }

    /// Virtualized views release offscreen pages; expired entry IDs cannot be
    /// reused for operations. A directory's total size is not a cache budget.
    pub fn release_entries(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        entries: &[RemoteEntryId],
        now: Instant,
    ) -> Result<(), SftpError> {
        if entries.len() > DIRECTORY_PAGE_ENTRIES {
            return Err(SftpError::ResourceLimit);
        }
        let scope = self.scope(id, binding, now)?;
        for entry in entries {
            scope.entries.remove(entry);
        }
        Ok(())
    }

    pub fn resolve_entry(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        entry: RemoteEntryId,
        now: Instant,
    ) -> Result<(RemotePath, Metadata), SftpError> {
        self.scope(id, binding, now)?
            .entries
            .get(&entry)
            .cloned()
            .ok_or(SftpError::ScopeExpired)
    }

    pub fn detach(
        &mut self,
        id: SftpSessionId,
        binding: &Binding,
        now: Instant,
    ) -> Result<(), SftpError> {
        self.scope(id, binding, now)?;
        self.scopes.remove(&id);
        Ok(())
    }

    pub fn revoke_terminal(&mut self, terminal: Uuid) {
        self.scopes
            .retain(|_, scope| scope.binding.terminal != terminal);
    }

    pub fn revoke_connection(&mut self, connection: Uuid, generation: u64) {
        self.scopes.retain(|_, scope| {
            scope.binding.connection != connection || scope.binding.generation != generation
        });
    }
}
