//! Actual production pool/service/file I/O against a fault-injectable wire peer.
use super::*;
use capstan_core::transfer::{
    Decision, TransferService, TransferSnapshot, TransferState, TransferTarget,
};
use std::sync::Mutex;
#[cfg(windows)]
#[path = "connection_recovery.rs"]
mod connection_recovery;
#[cfg(windows)]
#[path = "download_recovery.rs"]
mod download_recovery;
#[cfg(windows)]
#[path = "folder_conflicts.rs"]
mod folder_conflicts;
#[cfg(windows)]
#[path = "mixed_load.rs"]
mod mixed_load;
#[cfg(windows)]
#[path = "native_transfers.rs"]
mod native_transfers;
#[path = "remote_edits.rs"]
mod remote_edits;
#[path = "shell_transactions.rs"]
mod shell_transactions;
#[path = "tree_delete.rs"]
mod tree_delete;
#[cfg(windows)]
#[path = "tree_plans.rs"]
mod tree_plans;

#[derive(Clone)]
struct Record {
    data: Vec<u8>,
    mode: u32,
    mtime: u32,
}
#[derive(Default)]
pub(super) struct StatBatches {
    pub(super) groups: std::collections::VecDeque<usize>,
    pub(super) observed: Vec<String>,
    pub(super) peak: usize,
    pub(super) denied: std::collections::HashSet<String>,
}
#[derive(Default)]
pub(super) struct TreeParentBatch {
    pub(super) canonical_missing: bool,
    pub(super) created: Vec<String>,
    pub(super) paths: std::collections::HashSet<String>,
    pub(super) observed: Vec<String>,
    pub(super) canonical: Vec<String>,
    pub(super) peak: usize,
    pub(super) denied: Option<String>,
    pub(super) replaced: Option<String>,
}
#[derive(Default)]
pub(super) struct RemoteState {
    files: Mutex<HashMap<String, Record>>,
    pub(super) stat_batches: Mutex<StatBatches>,
    pub(super) tree_parent_batch: Mutex<TreeParentBatch>,
    metadata_batch: Mutex<MetadataBatch>,
    without_extensions: AtomicBool,
    deny_create: AtomicBool,
    deny_write: AtomicBool,
    deny_commit: AtomicBool,
    replace_denied_temporary: AtomicBool,
    lose_commit_ack: AtomicBool,
    pub(super) commits: AtomicUsize,
    corrupt_read: AtomicBool,
    pub(super) stall_write: AtomicBool,
    stall_read: AtomicBool,
    stall_fsync: AtomicBool,
    stall_realpath: AtomicBool,
    denied_removals: Mutex<std::collections::HashSet<String>>,
    stall_remove: AtomicBool,
    pub(super) started: Notify,
    pub(super) release: Notify,
}
#[derive(Default)]
struct MetadataBatch {
    initial: bool,
    publication: bool,
    temporary_stats: usize,
    observed: Vec<Vec<(u8, String)>>,
    denied_final_parent: bool,
    replace_final_temporary: bool,
}
impl RemoteState {
    pub(super) fn put(&self, path: &str, data: &[u8]) {
        let mut files = self.files.lock().unwrap();
        let mtime = files.get(path).map_or(100, |record| record.mtime + 1);
        files.insert(
            path.into(),
            Record {
                data: data.to_vec(),
                mode: 0o100600,
                mtime,
            },
        );
    }
    pub(super) fn bytes(&self, path: &str) -> Vec<u8> {
        self.files.lock().unwrap()[path].data.clone()
    }
    pub(super) fn no_partials(&self) -> bool {
        !self
            .files
            .lock()
            .unwrap()
            .keys()
            .any(|name| name.contains(".partial"))
    }
    async fn gate(&self, flag: &AtomicBool) {
        if flag.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.release.notified().await;
        }
    }
}
fn take_string(packet: &[u8], cursor: &mut usize) -> Vec<u8> {
    let length = u32::from_be_bytes(packet[*cursor..*cursor + 4].try_into().unwrap()) as usize;
    *cursor += 4;
    let value = packet[*cursor..*cursor + length].to_vec();
    *cursor += length;
    value
}
fn attrs(id: u32, record: &Record) -> Vec<u8> {
    let mut packet = reply(105, id);
    packet.extend_from_slice(&13_u32.to_be_bytes());
    packet.extend_from_slice(&(record.data.len() as u64).to_be_bytes());
    packet.extend_from_slice(&record.mode.to_be_bytes());
    packet.extend_from_slice(&record.mtime.to_be_bytes());
    packet.extend_from_slice(&record.mtime.to_be_bytes());
    packet
}
pub(super) async fn serve<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
>(
    stream: S,
    observations: Arc<Observations>,
) {
    let (mut read, mut write) = tokio::io::split(stream);
    // Read and write independently: three pipelined peers can otherwise fill
    // both bounded russh message queues while this fixture awaits a reply
    // write, preventing the transport loop from delivering more requests.
    let (responses, mut receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(208);
    let mut writers = tokio::task::JoinSet::new();
    writers.spawn(async move {
        while let Some(response) = receiver.recv().await {
            if write.write_u32(response.len() as u32).await.is_err()
                || write.write_all(&response).await.is_err()
            {
                break;
            }
        }
    });
    let state = &observations.transfer;
    let mut handles = HashMap::<String, (String, bool)>::new();
    let mut gauge = OpenHandles::new(&observations);
    let mut serial = 0;
    let mut held_stats = Vec::new();
    let mut held_parents = Vec::new();
    let mut held_metadata = Vec::new();
    let mut metadata_paths = Vec::new();
    'requests: while let Ok(length) = read.read_u32().await {
        assert!(length <= 262144);
        let mut packet = vec![0; length as usize];
        if read.read_exact(&mut packet).await.is_err() {
            break;
        }
        let id = u32::from_be_bytes(packet[1..5].try_into().unwrap());
        let mut cursor = 5;
        let path = if packet[0] == 1 {
            String::new()
        } else {
            String::from_utf8(take_string(&packet, &mut cursor)).unwrap()
        };
        let stat_path = (packet[0] == 7).then(|| path.clone());
        let metadata_path = matches!(packet[0], 7 | 16).then(|| path.clone());
        let final_metadata = state.metadata_batch.lock().unwrap().temporary_stats >= 2;
        if final_metadata && packet[0] == 16 {
            state
                .tree_parent_batch
                .lock()
                .unwrap()
                .canonical
                .push(path.clone());
        }
        if final_metadata
            && packet[0] == 7
            && state.tree_parent_batch.lock().unwrap().replaced.as_deref() == Some(path.as_str())
        {
            state.put(&path, b"replaced ancestor");
        }
        let deny_final_parent = final_metadata
            && packet[0] == 16
            && state.metadata_batch.lock().unwrap().denied_final_parent;
        if final_metadata
            && packet[0] == 7
            && path.ends_with(".partial")
            && state.metadata_batch.lock().unwrap().replace_final_temporary
        {
            state.put(&path, b"changed after full hash");
        }
        let response = match packet[0] {
            1 => {
                let mut response = vec![2, 0, 0, 0, 3];
                if !state.without_extensions.load(Ordering::SeqCst) {
                    for extension in ["posix-rename@openssh.com", "fsync@openssh.com"] {
                        string(&mut response, extension.as_bytes());
                        string(&mut response, b"1");
                    }
                }
                response
            }
            16 => {
                state.gate(&state.stall_realpath).await;
                if deny_final_parent {
                    status(id, 3)
                } else if state.tree_parent_batch.lock().unwrap().canonical_missing
                    && path != "/files"
                    && path != "."
                    && !state.files.lock().unwrap().contains_key(&path)
                {
                    status(id, 2)
                } else {
                    names(
                        id,
                        vec![(
                            if path == "." {
                                b"/files".to_vec()
                            } else {
                                path.into_bytes()
                            },
                            0o40755,
                        )],
                    )
                }
            }
            11 => {
                serial += 1;
                let handle = format!("d{serial}");
                handles.insert(handle.clone(), (path, false));
                gauge.opened();
                let mut response = reply(102, id);
                string(&mut response, handle.as_bytes());
                response
            }
            12 => {
                let (directory, ended) = handles.get_mut(&path).unwrap();
                if *ended {
                    status(id, 1)
                } else {
                    *ended = true;
                    let files = state.files.lock().unwrap();
                    let entries: Vec<_> = files
                        .iter()
                        .filter(|(path, _)| {
                            path.strip_prefix(&format!("{directory}/"))
                                .is_some_and(|relative| !relative.contains('/'))
                        })
                        .collect();
                    let mut response = reply(104, id);
                    response.extend_from_slice(&(entries.len() as u32).to_be_bytes());
                    for (path, record) in entries {
                        string(&mut response, path.rsplit('/').next().unwrap().as_bytes());
                        string(&mut response, b"");
                        response.extend_from_slice(&attrs(0, record)[5..]);
                    }
                    response
                }
            }
            7 | 8 => {
                let name = if packet[0] == 8 {
                    &handles[&path].0
                } else {
                    &path
                };
                if state.stat_batches.lock().unwrap().denied.contains(name) {
                    status(id, 3)
                } else {
                    let files = state.files.lock().unwrap();
                    files
                        .get(name)
                        .map_or_else(|| status(id, 2), |record| attrs(id, record))
                }
            }
            3 => {
                let flags = u32::from_be_bytes(packet[cursor..cursor + 4].try_into().unwrap());
                let mut files = state.files.lock().unwrap();
                if flags & 8 != 0 && state.deny_create.load(Ordering::SeqCst) {
                    status(id, 3)
                } else if flags & 8 != 0 && files.contains_key(&path) {
                    status(id, 4)
                } else if flags & 8 == 0 && !files.contains_key(&path) {
                    status(id, 2)
                } else {
                    if flags & 8 != 0 {
                        files.insert(
                            path.clone(),
                            Record {
                                data: vec![],
                                mode: 0o100600,
                                mtime: 100,
                            },
                        );
                    }
                    serial += 1;
                    let handle = format!("f{serial}");
                    handles.insert(handle.clone(), (path, false));
                    gauge.opened();
                    let mut response = reply(102, id);
                    string(&mut response, handle.as_bytes());
                    response
                }
            }
            5 | 6 => {
                let offset =
                    u64::from_be_bytes(packet[cursor..cursor + 8].try_into().unwrap()) as usize;
                cursor += 8;
                let name = &handles[&path].0;
                if packet[0] == 6 {
                    state.gate(&state.stall_write).await;
                    if state.deny_write.load(Ordering::SeqCst) {
                        status(id, 3)
                    } else {
                        let data = take_string(&packet, &mut cursor);
                        assert!(data.len() <= 32768);
                        let mut files = state.files.lock().unwrap();
                        let record = files.get_mut(name).unwrap();
                        record
                            .data
                            .resize(record.data.len().max(offset + data.len()), 0);
                        record.data[offset..offset + data.len()].copy_from_slice(&data);
                        status(id, 0)
                    }
                } else {
                    if offset >= 256 * 1024 {
                        state.gate(&state.stall_read).await;
                    }
                    let count =
                        u32::from_be_bytes(packet[cursor..cursor + 4].try_into().unwrap()) as usize;
                    assert!(count <= 32768);
                    let files = state.files.lock().unwrap();
                    let record = &files[name];
                    if offset >= record.data.len() {
                        status(id, 1)
                    } else {
                        // Exercise short reads independently of request size.
                        let mut data = record.data
                            [offset..(offset + count.min(7919)).min(record.data.len())]
                            .to_vec();
                        if state.corrupt_read.load(Ordering::SeqCst) && name.contains(".partial") {
                            data[0] ^= 1;
                        }
                        let mut response = reply(103, id);
                        string(&mut response, &data);
                        response
                    }
                }
            }
            4 => {
                assert!(handles.remove(&path).is_some());
                gauge.closed();
                status(id, 0)
            }
            13 | 15 => {
                state.gate(&state.stall_remove).await;
                let mut files = state.files.lock().unwrap();
                if state.denied_removals.lock().unwrap().contains(&path) {
                    status(id, 3)
                } else if packet[0] == 15
                    && files
                        .keys()
                        .any(|child| child.starts_with(&format!("{path}/")))
                {
                    status(id, 4)
                } else {
                    files.remove(&path);
                    status(id, 0)
                }
            }
            14 => {
                if state.tree_parent_batch.lock().unwrap().canonical_missing {
                    state
                        .tree_parent_batch
                        .lock()
                        .unwrap()
                        .created
                        .push(path.clone());
                }
                let mut files = state.files.lock().unwrap();
                if let std::collections::hash_map::Entry::Vacant(entry) = files.entry(path) {
                    entry.insert(Record {
                        data: Vec::new(),
                        mode: 0o40700,
                        mtime: 100,
                    });
                    status(id, 0)
                } else {
                    status(id, 4)
                }
            }
            9 => {
                let flags = u32::from_be_bytes(packet[cursor..cursor + 4].try_into().unwrap());
                cursor += 4;
                assert_eq!(flags, 4);
                let permissions =
                    u32::from_be_bytes(packet[cursor..cursor + 4].try_into().unwrap());
                let mut files = state.files.lock().unwrap();
                files.get_mut(&path).unwrap().mode = 0o100000 | permissions;
                status(id, 0)
            }
            18 | 200 => {
                if path == "fsync@openssh.com" {
                    state.gate(&state.stall_fsync).await;
                    status(id, 0)
                } else {
                    let (source, target) = if packet[0] == 18 {
                        (
                            path.clone(),
                            String::from_utf8(take_string(&packet, &mut cursor)).unwrap(),
                        )
                    } else {
                        assert_eq!(path, "posix-rename@openssh.com");
                        (
                            String::from_utf8(take_string(&packet, &mut cursor)).unwrap(),
                            String::from_utf8(take_string(&packet, &mut cursor)).unwrap(),
                        )
                    };
                    let mut files = state.files.lock().unwrap();
                    if state.deny_commit.load(Ordering::SeqCst) {
                        if state.replace_denied_temporary.load(Ordering::SeqCst) {
                            let record = files.get_mut(&source).unwrap();
                            record.data = b"external replacement".to_vec();
                            record.mtime += 1;
                        }
                        status(id, 3)
                    } else if packet[0] == 18 && files.contains_key(&target) {
                        status(id, 4)
                    } else {
                        let file = files.remove(&source).unwrap();
                        files.insert(target, file);
                        state.commits.fetch_add(1, Ordering::SeqCst);
                        if state.lose_commit_ack.load(Ordering::SeqCst) {
                            break;
                        }
                        status(id, 0)
                    }
                }
            }
            other => panic!("unexpected transfer request {other}"),
        };
        if final_metadata {
            if let Some(path) = stat_path.as_ref() {
                let released = {
                    let mut batch = state.tree_parent_batch.lock().unwrap();
                    if batch.paths.contains(path) {
                        batch.observed.push(path.clone());
                        held_parents.push(if batch.denied.as_deref() == Some(path.as_str()) {
                            status(id, 3)
                        } else {
                            response.clone()
                        });
                        batch.peak = batch.peak.max(held_parents.len());
                        Some(if held_parents.len() == batch.paths.len() {
                            std::mem::take(&mut held_parents)
                        } else {
                            Vec::new()
                        })
                    } else {
                        None
                    }
                };
                if let Some(released) = released {
                    for response in released.into_iter().rev() {
                        if responses.send(response).await.is_err() {
                            break 'requests;
                        }
                    }
                    continue;
                }
            }
        }
        if let Some(path) = metadata_path {
            let count = {
                let mut batch = state.metadata_batch.lock().unwrap();
                let count = if batch.initial {
                    2
                } else if batch.publication && final_metadata {
                    3
                } else {
                    0
                };
                if packet[0] == 7 && path.ends_with(".partial") {
                    batch.temporary_stats += 1;
                }
                count
            };
            if count > 0 {
                held_metadata.push(response);
                metadata_paths.push((packet[0], path));
                if held_metadata.len() == count {
                    {
                        let mut batch = state.metadata_batch.lock().unwrap();
                        batch.observed.push(std::mem::take(&mut metadata_paths));
                        if batch.initial {
                            batch.initial = false;
                        } else {
                            batch.publication = false;
                        }
                    }
                    for response in std::mem::take(&mut held_metadata).into_iter().rev() {
                        if responses.send(response).await.is_err() {
                            break 'requests;
                        }
                    }
                }
                continue;
            }
        }
        if let Some(path) = stat_path {
            let released = {
                let mut batches = state.stat_batches.lock().unwrap();
                batches.observed.push(path);
                if let Some(&size) = batches.groups.front() {
                    held_stats.push(response.clone());
                    batches.peak = batches.peak.max(held_stats.len());
                    Some(if held_stats.len() == size {
                        batches.groups.pop_front();
                        std::mem::take(&mut held_stats)
                    } else {
                        Vec::new()
                    })
                } else {
                    None
                }
            };
            if let Some(released) = released {
                // Replies are withheld until the whole bounded batch arrives,
                // then delivered in reverse order to detect index mix-ups.
                for response in released.into_iter().rev() {
                    if responses.send(response).await.is_err() {
                        break 'requests;
                    }
                }
                continue;
            }
        }
        if responses.send(response).await.is_err() {
            break;
        }
    }
    drop(responses);
    let _ = writers.join_next().await;
}

async fn fixture() -> (Fixture, SftpSessionId, Arc<TransferService>) {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    (fixture, session, Arc::new(TransferService::default()))
}
pub(super) fn upload_context(
    fixture: &Fixture,
    session: SftpSessionId,
    name: &str,
) -> capstan_core::transfer::TransferContext {
    fixture
        .service
        .transfer_context(
            "main",
            "one",
            session,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: name.into(),
            },
        )
        .unwrap()
}
pub(super) async fn wait(
    service: &TransferService,
    id: Uuid,
    predicate: impl Fn(&TransferSnapshot) -> bool,
) -> TransferSnapshot {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let snapshot = service
                .snapshot("main")
                .unwrap()
                .into_iter()
                .find(|snapshot| snapshot.id == id)
                .unwrap();
            if predicate(&snapshot) {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "transfer {id} did not reach expected state: {:?}",
            service.snapshot("main")
        )
    })
}
pub(super) async fn finished(service: &TransferService, id: Uuid) -> TransferSnapshot {
    wait(service, id, |s| {
        matches!(
            s.state,
            TransferState::Completed
                | TransferState::Failed
                | TransferState::Canceled
                | TransferState::Interrupted
        )
    })
    .await
}
#[cfg(windows)]
pub(super) fn enqueue_upload(
    fixture: &Fixture,
    session: SftpSessionId,
    transfers: &Arc<TransferService>,
    source: &std::path::Path,
    name: &str,
) -> Uuid {
    let context = upload_context(fixture, session, name);
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(source).unwrap()),
        )
        .unwrap();
    transfers.enqueue(context, selection.token).unwrap()
}

#[cfg(windows)]
#[tokio::test]
async fn native_files_roundtrip_with_short_reads_empty_files_and_full_hashes() {
    use sha2::{Digest, Sha256};
    let (fixture, session, transfers) = fixture().await;
    let root = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        (
            "中文 space.bin",
            (0..160_001).map(|n| (n * 31) as u8).collect::<Vec<_>>(),
        ),
        ("empty", vec![]),
    ] {
        let source = root.path().join(format!("source-{name}"));
        std::fs::write(&source, &bytes).unwrap();
        let id = enqueue_upload(&fixture, session, &transfers, &source, name);
        assert_eq!(
            finished(&transfers, id).await.state,
            TransferState::Completed
        );
        assert_eq!(
            Sha256::digest(
                fixture
                    .observations
                    .transfer
                    .bytes(&format!("/files/{name}"))
            ),
            Sha256::digest(&bytes)
        );
        let page = fixture.list(session, "/files").await.unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap();
        let context = fixture
            .service
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::Download { entry: entry.id },
            )
            .unwrap();
        let output = root.path().join(format!("output-{name}"));
        let local = capstan_core::transfer::WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&output).unwrap(),
        );
        let selection = transfers
            .grant_destination(&context, Arc::new(local))
            .unwrap();
        let id = transfers.enqueue(context, selection.token).unwrap();
        let status = finished(&transfers, id).await;
        assert_eq!(status.state, TransferState::Completed, "{status:?}");
        assert_eq!(
            Sha256::digest(std::fs::read(&output).unwrap()),
            Sha256::digest(&bytes)
        );
        assert_eq!(status.processed, status.total);
    }
    assert!(fixture.observations.transfer.no_partials());
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn conflict_decision_is_single_use_and_rechecked_after_write() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/target", b"original");
    remote.stall_fsync.store(true, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new content").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let first = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert_eq!(
        transfers.resolve("other", id, first.revision, Decision::Replace),
        Err(SftpError::ScopeExpired)
    );
    remote.put("/files/target", b"changed while deciding");
    transfers
        .resolve("main", id, first.revision, Decision::Replace)
        .unwrap();
    let second = wait(&transfers, id, |s| {
        s.conflict
            .as_ref()
            .is_some_and(|c| c.revision != first.revision)
    })
    .await
    .conflict
    .unwrap();
    assert_eq!(
        transfers.resolve("main", id, first.revision, Decision::Replace),
        Err(SftpError::ScopeExpired)
    );
    transfers
        .resolve("main", id, second.revision, Decision::Replace)
        .unwrap();
    remote.started.notified().await;
    remote.put("/files/target", b"changed after write");
    remote.stall_fsync.store(false, Ordering::SeqCst);
    remote.release.notify_one();
    let third = wait(&transfers, id, |s| s.conflict.is_some()).await;
    assert_eq!(third.processed, third.total);
    let third = third.conflict.unwrap();
    assert_eq!(remote.bytes("/files/target"), b"changed after write");
    transfers
        .resolve(
            "main",
            id,
            third.revision,
            Decision::Rename {
                name: "kept-copy".into(),
            },
        )
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(remote.bytes("/files/target"), b"changed after write");
    assert_eq!(remote.bytes("/files/kept-copy"), b"new content");
    assert!(remote.no_partials());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn missing_extensions_refuse_replacement_and_allow_explicit_save_as() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .without_extensions
        .store(true, Ordering::SeqCst);
    fixture
        .observations
        .transfer
        .put("/files/target", b"original");
    let session = fixture.attach("one").await;
    fixture.list(session, "/files").await.unwrap();
    let transfers = Arc::new(TransferService::default());
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert!(!conflict.replace_allowed);
    assert_eq!(
        transfers.resolve("main", id, conflict.revision, Decision::Replace),
        Err(SftpError::UnsupportedFileType)
    );
    transfers
        .resolve(
            "main",
            id,
            conflict.revision,
            Decision::Rename {
                name: "copy".into(),
            },
        )
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(
        fixture.observations.transfer.bytes("/files/target"),
        b"original"
    );
    assert_eq!(fixture.observations.transfer.bytes("/files/copy"), b"new");
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn remote_write_commit_and_hash_failures_preserve_targets_and_remove_partials() {
    for fault in 0..3 {
        let (fixture, session, transfers) = fixture().await;
        let remote = &fixture.observations.transfer;
        remote.put("/files/target", b"original");
        match fault {
            0 => remote.deny_write.store(true, Ordering::SeqCst),
            1 => remote.deny_commit.store(true, Ordering::SeqCst),
            _ => remote.corrupt_read.store(true, Ordering::SeqCst),
        }
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"new").unwrap();
        let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
        let conflict = wait(&transfers, id, |s| s.conflict.is_some())
            .await
            .conflict
            .unwrap();
        transfers
            .resolve("main", id, conflict.revision, Decision::Replace)
            .unwrap();
        let status = finished(&transfers, id).await;
        assert_eq!(status.state, TransferState::Failed, "{status:?}");
        assert_eq!(
            status.error,
            Some(if fault == 2 {
                SftpError::RemoteIo
            } else {
                SftpError::PermissionDenied
            })
        );
        assert!(!status.cleanup_required);
        assert_eq!(remote.bytes("/files/target"), b"original");
        assert!(remote.no_partials());
        fixture.close().await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn failed_upload_cleanup_preserves_a_replaced_remote_temporary() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/target", b"original");
    remote.deny_commit.store(true, Ordering::SeqCst);
    remote
        .replace_denied_temporary
        .store(true, Ordering::SeqCst);
    let recovery = tempfile::tempdir().unwrap();
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::Replace)
        .unwrap();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed, "{status:?}");
    assert_eq!(status.error, Some(SftpError::PermissionDenied));
    assert!(status.cleanup_required);
    assert_eq!(remote.bytes("/files/target"), b"original");
    {
        let files = remote.files.lock().unwrap();
        let partials: Vec<_> = files
            .iter()
            .filter(|(name, _)| name.contains(".partial"))
            .collect();
        assert_eq!(partials.len(), 1);
        assert_eq!(partials[0].1.data, b"external replacement");
    }
    let records = transfers.recovery_snapshot().unwrap();
    assert_eq!(records.entries.len(), 1);
    assert_eq!(
        records.entries[0].state,
        capstan_core::transfer::RecoveryState::ManualInspection
    );
    assert_eq!(
        transfers
            .cleanup_recovered_upload("main", id, upload_context(&fixture, session, "target"))
            .await,
        Err(SftpError::ResumeMismatch)
    );
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn failed_owned_upload_cleanup_keeps_a_baseline_bound_cleanup_record() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.deny_commit.store(true, Ordering::SeqCst);
    remote.stall_write.store(true, Ordering::SeqCst);
    let recovery = tempfile::tempdir().unwrap();
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let partial = format!("/files/.capstan-{id}.partial");
    tokio::time::timeout(Duration::from_secs(3), remote.started.notified())
        .await
        .unwrap();
    remote
        .denied_removals
        .lock()
        .unwrap()
        .insert(partial.clone());
    remote.stall_write.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed);
    assert!(status.cleanup_required);
    let snapshot = transfers.recovery_snapshot().unwrap();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(
        snapshot.entries[0].state,
        capstan_core::transfer::RecoveryState::CleanupRequired
    );
    assert_eq!(snapshot.entries[0].partial_size.unwrap().0, 3);
    drop(transfers);
    let transfers = Arc::new(TransferService::default());
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    assert_eq!(
        transfers.recovery_snapshot().unwrap().entries[0].state,
        capstan_core::transfer::RecoveryState::CleanupRequired
    );
    let context = upload_context(&fixture, session, "target");
    let grant = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    assert_eq!(
        transfers.retry_recovered_upload("main", id, context, grant.token),
        Err(SftpError::ScopeExpired)
    );
    let baseline = remote.files.lock().unwrap()[&partial].clone();
    remote.denied_removals.lock().unwrap().remove(&partial);
    remote.put(&partial, b"bad");
    assert_eq!(
        transfers
            .cleanup_recovered_upload("main", id, upload_context(&fixture, session, "target"))
            .await,
        Err(SftpError::ResumeMismatch)
    );
    assert_eq!(remote.bytes(&partial), b"bad");
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1);
    remote.files.lock().unwrap().insert(partial, baseline);
    transfers
        .cleanup_recovered_upload("main", id, upload_context(&fixture, session, "target"))
        .await
        .unwrap();
    assert!(remote.no_partials());
    assert!(transfers.recovery_snapshot().unwrap().entries.is_empty());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn rejected_exclusive_open_does_not_report_a_nonexistent_partial() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.deny_create.store(true, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed, "{status:?}");
    assert_eq!(status.error, Some(SftpError::PermissionDenied));
    assert!(!status.cleanup_required);
    assert!(remote.no_partials());
    assert_eq!(remote.commits.load(Ordering::SeqCst), 0);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn lost_commit_ack_is_verified_read_only_on_the_same_transport_without_replay() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/target", b"original");
    remote.lose_commit_ack.store(true, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"verified new content").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::Replace)
        .unwrap();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Completed, "{status:?}");
    assert!(!status.cleanup_required);
    assert_eq!(remote.bytes("/files/target"), b"verified new content");
    assert_eq!(remote.commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    assert!(remote.no_partials());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn task_lease_survives_originating_terminal_revocation() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.stall_fsync.store(true, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"still authorized").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    remote.started.notified().await;
    fixture.service.revoke_terminal("one");
    remote.stall_fsync.store(false, Ordering::SeqCst);
    remote.release.notify_one();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    assert_eq!(remote.bytes("/files/target"), b"still authorized");
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 1);
    fixture.attach("two").await;
    // Completion releases the last SFTP user; the weak cache does not keep a
    // subsystem alive indefinitely. Reattachment opens one new subsystem.
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 2);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn three_worker_limit_queued_cancel_and_inflight_cancel_keep_targets() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.stall_write.store(true, Ordering::SeqCst);
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let mut ids = vec![];
    for n in 0..5 {
        ids.push(enqueue_upload(
            &fixture,
            session,
            &transfers,
            &source,
            &format!("target-{n}"),
        ));
    }
    remote.started.notified().await;
    wait(&transfers, ids[2], |s| s.state == TransferState::Running).await;
    #[cfg(feature = "benchmarks")]
    {
        let (current, peak) = transfers.buffer_usage();
        assert!(current > 0 && current <= peak);
        assert!(peak <= capstan_core::sftp::MAX_BUFFER_BYTES);
    }
    let rows = transfers.snapshot("main").unwrap();
    assert_eq!(
        rows.iter()
            .filter(|s| s.state == TransferState::Running)
            .count(),
        3
    );
    assert_eq!(
        rows.iter()
            .filter(|s| s.state == TransferState::Queued)
            .count(),
        2
    );
    transfers.cancel("main", ids[4]).unwrap();
    assert_eq!(
        finished(&transfers, ids[4]).await.state,
        TransferState::Canceled
    );
    transfers.cancel("main", ids[0]).unwrap();
    let status = finished(&transfers, ids[0]).await;
    assert_eq!(status.state, TransferState::Canceled);
    assert!(status.cleanup_required);
    // Request cancellation is isolated to the canceled job. Sibling jobs keep
    // using their own subsystems and may complete after the stalled peer is
    // released.
    remote.stall_write.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    for id in &ids[1..4] {
        assert_eq!(
            finished(&transfers, *id).await.state,
            TransferState::Completed
        );
    }
    #[cfg(feature = "benchmarks")]
    assert_eq!(
        transfers.buffer_usage().0,
        0,
        "canceled/queued jobs retain no payload admission"
    );
    assert!(remote
        .files
        .lock()
        .unwrap()
        .keys()
        .any(|name| name.contains("/target-1")));
    fixture.attach("two").await;
    assert_eq!(fixture.observations.auth.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observations.subsystems.load(Ordering::SeqCst), 3);
    assert_eq!(
        fixture.observations.peak_subsystems.load(Ordering::SeqCst),
        3
    );
    fixture.close().await;
}

#[tokio::test]
async fn revocation_during_attachment_is_prompt_and_does_not_register_a_view() {
    let fixture = Fixture::new().await;
    fixture
        .observations
        .transfer_mode
        .store(true, Ordering::SeqCst);
    let remote = &fixture.observations.transfer;
    remote.stall_realpath.store(true, Ordering::SeqCst);
    let cancel = CancellationToken::new();
    {
        let attach = fixture.service.attach("main", "one", &cancel);
        tokio::pin!(attach);
        tokio::select! {
            result = &mut attach => panic!("attachment unexpectedly finished: {result:?}"),
            () = remote.started.notified() => {},
        }
        fixture.service.revoke_terminal("one");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), attach)
                .await
                .unwrap()
                .unwrap_err(),
            SftpError::ScopeExpired
        );
    }
    remote.stall_realpath.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    fixture.close().await;
}

#[cfg(windows)]
pub(super) struct FaultDestination {
    pub(super) inner: capstan_core::transfer::WindowsDestination,
    pub(super) fault: u8,
    pub(super) hold: Option<Arc<CommitHold>>,
}
#[cfg(windows)]
#[derive(Default)]
pub(super) struct CommitHold {
    entered: AtomicUsize,
    pub(super) started: Notify,
    pub(super) released: Mutex<bool>,
    pub(super) changed: std::sync::Condvar,
}
#[cfg(windows)]
impl capstan_core::transfer::LocalDestination for FaultDestination {
    fn name(&self) -> Result<String, SftpError> {
        self.inner.name()
    }
    fn snapshot(&self) -> Result<Option<capstan_core::transfer::LocalVersion>, SftpError> {
        self.inner.snapshot()
    }
    fn retarget(&self, name: &str) -> Result<(), SftpError> {
        self.inner.retarget(name)
    }
    fn begin(&self, name: &str) -> Result<(), SftpError> {
        self.inner.begin(name)
    }
    fn target_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        if self.fault == 4 {
            self.inner.target_path()
        } else {
            Ok(None)
        }
    }
    fn partial_path(&self) -> Result<Option<std::path::PathBuf>, SftpError> {
        if self.fault == 4 {
            self.inner.partial_path()
        } else {
            Ok(None)
        }
    }
    fn partial_size(&self) -> Result<Option<u64>, SftpError> {
        if self.fault == 4 {
            self.inner.partial_size()
        } else {
            Ok(None)
        }
    }
    fn partial_identity(&self) -> Result<Option<u128>, SftpError> {
        if self.fault == 4 {
            self.inner.partial_identity()
        } else {
            Ok(None)
        }
    }
    fn preserve_partial(&self) -> Result<(), SftpError> {
        self.inner.preserve_partial()
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), SftpError> {
        if self.fault == 0 {
            Err(SftpError::LocalIo)
        } else {
            self.inner.write(offset, data)
        }
    }
    fn read(&self, offset: u64, length: usize) -> Result<Vec<u8>, SftpError> {
        self.inner.read(offset, length)
    }
    fn sync(&self) -> Result<(), SftpError> {
        if self.fault == 1 {
            Err(SftpError::LocalIo)
        } else {
            self.inner.sync()
        }
    }
    fn commit(
        &self,
        expected: Option<capstan_core::transfer::LocalVersion>,
    ) -> Result<(), SftpError> {
        if self.fault == 2 || self.fault == 4 {
            Err(SftpError::PermissionDenied)
        } else {
            if let Some(hold) = &self.hold {
                hold.entered.fetch_add(1, Ordering::SeqCst);
                hold.started.notify_one();
                let released = hold.released.lock().unwrap();
                let (released, _) = hold
                    .changed
                    .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
                    .unwrap();
                if !*released {
                    return Err(SftpError::LocalIo);
                }
            }
            self.inner.commit(expected)
        }
    }
    fn discard(&self) -> Result<(), SftpError> {
        if self.fault == 4 {
            Err(SftpError::PermissionDenied)
        } else {
            self.inner.discard()
        }
    }
}

#[cfg(windows)]
#[tokio::test]
async fn failed_download_discard_keeps_an_identity_bound_cleanup_record() {
    use capstan_core::transfer::WindowsDestination;
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/source", b"remote content");
    let recovery = tempfile::tempdir().unwrap();
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
    let context = fixture
        .service
        .transfer_context("main", "one", session, TransferTarget::Download { entry })
        .unwrap();
    let grant = transfers
        .grant_destination(
            &context,
            Arc::new(FaultDestination {
                inner: WindowsDestination::new(
                    capstan_native_windows::transfer::Destination::select(&output).unwrap(),
                ),
                fault: 4,
                hold: None,
            }),
        )
        .unwrap();
    let id = transfers.enqueue(context.clone(), grant.token).unwrap();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed);
    assert!(status.cleanup_required);
    let partial = root.path().join(format!(".capstan-{id}.partial"));
    let snapshot = transfers.recovery_snapshot().unwrap();
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(
        snapshot.entries[0].state,
        capstan_core::transfer::RecoveryState::CleanupRequired
    );
    assert_eq!(std::fs::read(&partial).unwrap(), b"remote content");
    drop(transfers);
    let transfers = Arc::new(TransferService::default());
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    assert_eq!(
        transfers.recovery_snapshot().unwrap().entries[0].state,
        capstan_core::transfer::RecoveryState::CleanupRequired
    );
    let destination = Arc::new(WindowsDestination::new(
        capstan_native_windows::transfer::Destination::select(&output).unwrap(),
    ));
    let selection = transfers
        .grant_recovered_destination("main", id, &context, destination)
        .await
        .unwrap();
    transfers
        .cleanup_recovered_download("main", id, selection.token)
        .await
        .unwrap();
    assert!(!partial.exists());
    assert!(!output.exists());
    assert!(transfers.recovery_snapshot().unwrap().entries.is_empty());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn local_write_flush_and_commit_failures_keep_existing_files_and_clean_by_handle() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/source", b"remote content");
    let root = tempfile::tempdir().unwrap();
    for fault in 0..3 {
        let output = root.path().join(format!("output-{fault}"));
        std::fs::write(&output, b"keep original").unwrap();
        let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
        let context = fixture
            .service
            .transfer_context("main", "one", session, TransferTarget::Download { entry })
            .unwrap();
        let destination = FaultDestination {
            inner: capstan_core::transfer::WindowsDestination::new(
                capstan_native_windows::transfer::Destination::select(&output).unwrap(),
            ),
            fault,
            hold: None,
        };
        let selection = transfers
            .grant_destination(&context, Arc::new(destination))
            .unwrap();
        let id = transfers.enqueue(context, selection.token).unwrap();
        let conflict = wait(&transfers, id, |s| s.conflict.is_some())
            .await
            .conflict
            .unwrap();
        transfers
            .resolve("main", id, conflict.revision, Decision::Replace)
            .unwrap();
        let status = finished(&transfers, id).await;
        assert_eq!(status.state, TransferState::Failed, "{status:?}");
        assert!(!status.cleanup_required);
        assert_eq!(std::fs::read(&output).unwrap(), b"keep original");
        assert!(std::fs::read_dir(root.path()).unwrap().all(|item| !item
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".partial")));
    }
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn concurrent_download_publication_rechecks_the_target_after_the_previous_commit() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/source", b"remote content");
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    std::fs::write(&output, b"keep original").unwrap();
    let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
    let hold = Arc::new(CommitHold::default());
    let mut ids = Vec::new();
    for _ in 0..2 {
        let context = fixture
            .service
            .transfer_context("main", "one", session, TransferTarget::Download { entry })
            .unwrap();
        let destination = FaultDestination {
            inner: capstan_core::transfer::WindowsDestination::new(
                capstan_native_windows::transfer::Destination::select(&output).unwrap(),
            ),
            fault: 3,
            hold: Some(hold.clone()),
        };
        let selection = transfers
            .grant_destination(&context, Arc::new(destination))
            .unwrap();
        let id = transfers.enqueue(context, selection.token).unwrap();
        let conflict = wait(&transfers, id, |s| s.conflict.is_some())
            .await
            .conflict
            .unwrap();
        ids.push((id, conflict.revision));
    }
    for (id, revision) in &ids {
        transfers
            .resolve("main", *id, *revision, Decision::Replace)
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(4), hold.started.notified())
        .await
        .unwrap();
    let states = transfers.snapshot("main").unwrap();
    let first = states
        .iter()
        .find(|s| s.state == TransferState::Committing)
        .unwrap()
        .id;
    let second = ids.iter().find(|(id, _)| *id != first).unwrap();
    let waiting = wait(&transfers, second.0, |s| {
        matches!(
            s.state,
            TransferState::Verifying | TransferState::Committing
        )
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let entered = hold.entered.load(Ordering::SeqCst);
    *hold.released.lock().unwrap() = true;
    hold.changed.notify_all();
    assert_eq!(entered, 1, "formal target commits must not overlap");
    assert_eq!(waiting.processed, waiting.total);
    assert_eq!(
        finished(&transfers, first).await.state,
        TransferState::Completed
    );
    let conflict = wait(&transfers, second.0, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    assert_ne!(conflict.revision, second.1);
    transfers
        .resolve("main", second.0, conflict.revision, Decision::Skip)
        .unwrap();
    assert_eq!(
        finished(&transfers, second.0).await.state,
        TransferState::Canceled
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"remote content");
    assert_eq!(hold.entered.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn selection_tokens_are_owner_bound_single_use_and_releasable() {
    let (fixture, session, transfers) = fixture().await;
    let other = fixture.attach("two").await;
    fixture
        .service
        .list("main", "two", other, RemotePath::parse("/files").unwrap())
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"data").unwrap();
    let context = upload_context(&fixture, session, "target");
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    let other_context = fixture
        .service
        .transfer_context(
            "main",
            "two",
            other,
            TransferTarget::Upload {
                parent: RemotePath::parse("/files").unwrap(),
                name: "foreign".into(),
            },
        )
        .unwrap();
    assert_eq!(
        transfers.enqueue(other_context, selection.token),
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(
        transfers.release_selection("other-window", selection.token),
        Err(SftpError::ScopeExpired)
    );
    let id = transfers.enqueue(context, selection.token).unwrap();
    assert_eq!(
        transfers.enqueue(upload_context(&fixture, session, "again"), selection.token),
        Err(SftpError::ScopeExpired)
    );
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    let context = upload_context(&fixture, session, "unused");
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    transfers
        .release_selection("main", selection.token)
        .unwrap();
    assert!(std::fs::write(&source, b"released").is_ok());
    assert!(transfers.snapshot("other-window").unwrap().is_empty());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn skip_and_exit_drain_do_not_commit_or_leave_local_grants() {
    let (fixture, session, transfers) = fixture().await;
    fixture
        .observations
        .transfer
        .put("/files/target", b"original");
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::Skip)
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    wait(&transfers, id, |s| s.conflict.is_some()).await;
    assert!(transfers.has_active("main").unwrap());
    transfers.prepare_exit("main").await.unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Canceled
    );
    assert!(!transfers.has_active("main").unwrap());
    assert_eq!(
        fixture.observations.transfer.bytes("/files/target"),
        b"original"
    );
    assert!(fixture.observations.transfer.no_partials());
    assert!(std::fs::write(&source, b"released").is_ok());
    fixture.close().await;
}

#[cfg(windows)]
#[tokio::test]
async fn configured_openssh_core_file_workflow() {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let Ok(target) = std::env::var("CAPSTAN_TEST_SSHD") else {
        eprintln!("SKIP: CAPSTAN_TEST_SSHD is not configured (production core file workflow)");
        return;
    };
    let root = std::env::var("CAPSTAN_TEST_SFTP_ROOT").expect("an isolated root is required");
    assert!(
        root.starts_with("/var/tmp/capstan-m1-ssh-")
            && root.ends_with("/home")
            && !root.contains("..")
    );
    let password_file = std::env::var_os("CAPSTAN_TEST_SSH_PASSWORD_FILE").unwrap();
    let key_file = std::env::var_os("CAPSTAN_TEST_SSH_HOST_KEY_FILE").unwrap();
    let mut password = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(password_file)
        .unwrap()
        .take(8193)
        .read_to_string(&mut password)
        .unwrap();
    assert!(password.len() <= 8192);
    let password = secrecy::SecretString::from(password.trim_end_matches(['\r', '\n']).to_owned());
    let trust = Arc::new(Trust(
        HostPublicKey::from_openssh(&std::fs::read_to_string(key_file).unwrap()).unwrap(),
    ));
    let (username, endpoint) = target.split_once('@').unwrap();
    let (address, port) = endpoint.rsplit_once(':').unwrap();
    let host = Uuid::new_v4();
    let binding = AttemptBinding {
        attempt_id: Uuid::new_v4(),
        host_id: host,
        host_revision: 1,
        vault_epoch: 1,
        target: address.trim_start_matches('[').trim_end_matches(']').into(),
        port: port.parse().unwrap(),
    };
    let pool_key = PoolKey {
        target: binding.target.clone(),
        port: binding.port,
        username: username.into(),
        credential_id: Uuid::new_v4(),
        credential_revision: 1,
        trust_fingerprint: trust.0.fingerprint.clone(),
        vault_epoch: 1,
        agent_identity: None,
        agent_forwarding: false,
        proxy_path: String::new(),
    };
    let username = username.to_owned();
    let pool = ConnectionPool::<PasswordConnection>::new(Duration::from_secs(30));
    let (broker, _challenges) = TrustBroker::new();
    let lease = pool
        .acquire_checked(
            pool_key,
            CancellationToken::new(),
            |connection| !connection.is_closed(),
            move |cancel| async move {
                capstan_ssh::connect_password(
                    binding.target.clone(),
                    binding.port,
                    username,
                    password,
                    binding,
                    trust,
                    broker,
                    cancel,
                )
                .await
            },
        )
        .await
        .unwrap();
    let first = lease.open_shell(80, 24).await.unwrap();
    let second = lease.open_shell(80, 24).await.unwrap();
    assert_eq!(first.connection_id(), second.connection_id());
    let sftp = SftpService::default();
    sftp.register("main", "one", host, first.connection_id(), lease.clone())
        .unwrap();
    sftp.register("main", "two", host, second.connection_id(), lease.clone())
        .unwrap();
    let attach_cancel = CancellationToken::new();
    let (a, b) = tokio::join!(
        sftp.attach("main", "one", &attach_cancel),
        sftp.attach("main", "two", &attach_cancel)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.connection_generation, b.connection_generation);
    assert!(a.atomic_replace && a.fsync);
    assert_eq!(a.path.as_str(), root);
    sftp.list("main", "one", a.session_id, a.path.clone())
        .await
        .unwrap();
    let transfers = Arc::new(TransferService::default());
    let local = tempfile::tempdir().unwrap();
    let source = local.path().join("source.bin");
    let bytes: Vec<u8> = (0..1_048_613)
        .map(|n: u64| n.wrapping_mul(131).wrapping_add(n / 256) as u8)
        .collect();
    std::fs::write(&source, &bytes).unwrap();
    let remote_name = format!("capstan-core-{} 中文.bin", Uuid::new_v4());
    let context = sftp
        .transfer_context(
            "main",
            "one",
            a.session_id,
            TransferTarget::Upload {
                parent: a.path.clone(),
                name: remote_name.clone(),
            },
        )
        .unwrap();
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    // Replace through the production extension facade, with explicit consent.
    let context = sftp
        .transfer_context(
            "main",
            "one",
            a.session_id,
            TransferTarget::Upload {
                parent: a.path.clone(),
                name: remote_name.clone(),
            },
        )
        .unwrap();
    let selection = transfers
        .grant_source(
            &context,
            Arc::new(capstan_native_windows::transfer::Source::open(&source).unwrap()),
        )
        .unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    transfers
        .resolve("main", id, conflict.revision, Decision::Replace)
        .unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    let page = sftp
        .list("main", "one", a.session_id, a.path.clone())
        .await
        .unwrap();
    let entry = page
        .entries
        .iter()
        .find(|entry| entry.name == remote_name)
        .unwrap();
    let context = sftp
        .transfer_context(
            "main",
            "one",
            a.session_id,
            TransferTarget::Download { entry: entry.id },
        )
        .unwrap();
    let output = local.path().join("output.bin");
    let destination = capstan_core::transfer::WindowsDestination::new(
        capstan_native_windows::transfer::Destination::select(&output).unwrap(),
    );
    let selection = transfers
        .grant_destination(&context, Arc::new(destination))
        .unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    assert_eq!(
        finished(&transfers, id).await.state,
        TransferState::Completed
    );
    let digest = Sha256::digest(std::fs::read(output).unwrap());
    assert_eq!(digest, Sha256::digest(&bytes));
    let confirmation = sftp
        .prepare_mutation("main", "one", a.session_id, entry.id, Mutation::Delete)
        .await
        .unwrap();
    sftp.apply_mutation("main", "one", a.session_id, confirmation.token)
        .await
        .unwrap();
    let page = sftp
        .list("main", "one", a.session_id, a.path.clone())
        .await
        .unwrap();
    assert!(!page
        .entries
        .iter()
        .any(|entry| entry.name == remote_name || entry.name.contains(".partial")));
    let client = lease.open_sftp(&CancellationToken::new()).await.unwrap();
    let folder_batches = folder_conflicts::openssh_folder_batches(
        &sftp,
        a.session_id,
        &a.path,
        &client,
        local.path(),
        &transfers,
    )
    .await;
    // Only an owned UUID tree inside the explicitly configured isolated root.
    let delete_name = format!("capstan-delete-{}", Uuid::new_v4());
    let delete_root = a
        .path
        .join(&capstan_core::sftp::RemoteName::from_bytes(delete_name.as_bytes()).unwrap())
        .unwrap();
    let delete_sub = delete_root
        .join(&capstan_core::sftp::RemoteName::from_bytes(b"sub").unwrap())
        .unwrap();
    let delete_empty = delete_root
        .join(&capstan_core::sftp::RemoteName::from_bytes(b"empty").unwrap())
        .unwrap();
    let delete_file = delete_sub
        .join(&capstan_core::sftp::RemoteName::from_bytes(b"file").unwrap())
        .unwrap();
    let delete_cancel = CancellationToken::new();
    for directory in [&delete_root, &delete_sub, &delete_empty] {
        client
            .mkdir(directory.as_str(), &delete_cancel)
            .await
            .unwrap();
    }
    capstan_core::remote_edit::apply_remote_bytes(
        &client,
        &delete_file,
        b"isolated delete content",
        None,
        None,
        &transfers.publication_gate(),
        &delete_cancel,
    )
    .await
    .unwrap();
    let delete_page = sftp
        .list("main", "one", a.session_id, a.path.clone())
        .await
        .unwrap();
    let delete_entry = delete_page
        .entries
        .iter()
        .find(|entry| entry.name == delete_name)
        .unwrap();
    let delete_plan = sftp
        .prepare_tree_delete("main", "one", a.session_id, delete_entry.id)
        .await
        .unwrap();
    let delete_report = sftp
        .apply_tree_delete("main", "one", a.session_id, delete_plan.token)
        .await
        .unwrap();
    assert_eq!(delete_report.entries.len(), 4);
    assert!(delete_report
        .entries
        .iter()
        .all(|entry| entry.outcome == capstan_core::sftp::DeleteOutcome::Removed));
    assert!(matches!(
        client.lstat(delete_root.as_str(), &delete_cancel).await,
        Err(capstan_ssh::sftp::ClientError::NotFound)
    ));
    // Exercise the real editor publication path without launching an editor.
    // This is separate from desktop/save-monitor acceptance.
    let editor_path = a
        .path
        .join(
            &capstan_core::sftp::RemoteName::from_bytes(
                format!("capstan-editor-{}.txt", Uuid::new_v4()).as_bytes(),
            )
            .unwrap(),
        )
        .unwrap();
    let editor_original = b"\xEF\xBB\xBForiginal\r\n";
    let editor_changed = b"\xEF\xBB\xBFedited text\r\n";
    let gate = transfers.publication_gate();
    let cancel = CancellationToken::new();
    let editor_metadata = capstan_core::remote_edit::apply_remote_bytes(
        &client,
        &editor_path,
        editor_original,
        None,
        None,
        &gate,
        &cancel,
    )
    .await
    .unwrap();
    let original_hash: [u8; 32] = Sha256::digest(editor_original).into();
    let changed_metadata = capstan_core::remote_edit::apply_remote_bytes(
        &client,
        &editor_path,
        editor_changed,
        Some(&editor_metadata),
        Some(original_hash),
        &gate,
        &cancel,
    )
    .await
    .unwrap();
    let (editor_readback, _) = capstan_core::remote_edit::read_remote_file(
        &client,
        &editor_path,
        Some(&changed_metadata),
        &cancel,
    )
    .await
    .unwrap();
    assert_eq!(editor_readback, editor_changed);
    assert_eq!(
        capstan_core::remote_edit::apply_remote_bytes(
            &client,
            &editor_path,
            b"stale local edit",
            Some(&editor_metadata),
            Some(original_hash),
            &gate,
            &cancel,
        )
        .await,
        Err(capstan_core::remote_edit::EditError::Conflict)
    );
    client
        .remove_file(editor_path.as_str(), &cancel)
        .await
        .unwrap();
    let editor_digest = Sha256::digest(&editor_readback);
    client.close().await;
    assert_eq!(
        sftp.list("main", "one", a.session_id, a.path.clone()).await,
        Err(SftpError::ConnectionLost)
    );
    let reopened = sftp
        .attach("main", "one", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(reopened.connection_generation, a.connection_generation);
    let page = sftp
        .list("main", "one", reopened.session_id, a.path.clone())
        .await
        .unwrap();
    assert!(!page
        .entries
        .iter()
        .any(|entry| entry.name == remote_name || entry.name.contains(".partial")));
    sftp.detach("main", "one", reopened.session_id)
        .await
        .unwrap();
    sftp.detach("main", "one", a.session_id).await.unwrap();
    sftp.detach("main", "two", b.session_id).await.unwrap();
    sftp.revoke_terminal("one");
    sftp.revoke_terminal("two");
    first.control.close().await.unwrap();
    second.control.close().await.unwrap();
    for connection in pool.drain().await {
        connection.disconnect().await;
    }
    eprintln!(
        "SFTP_CORE_WORKFLOW {}",
        serde_json::json!({ "bytes": bytes.len().to_string(), "sha256": format!("{digest:x}"),
        "productionPoolAndService": true, "nativeLocalIo": true, "atomicReplace": true, "fsync": true,
        "checkedPoolAcquisition": true, "subsystemReopenOnSameTransport": true, "closedViewRejected": true,
        "remoteEditorPublication": true, "remoteEditorStaleBaselineRejected": true, "editorSha256": format!("{editor_digest:x}"),
        "folderBatches": folder_batches,
        "recursiveDelete": { "removed": delete_report.entries.len(), "emptyDirectoryPreservedUntilConfirmation": true, "knownTreeRemoved": true },
        "remoteCleanup": true, "performanceAcceptance": false, "desktopAcceptance": false })
    );
}

#[cfg(all(windows, feature = "benchmarks"))]
#[tokio::test]
async fn owned_payload_peak_stays_constant_when_native_file_size_doubles() {
    let (fixture, session, transfers) = fixture().await;
    let local = tempfile::tempdir().unwrap();
    let mut peaks = Vec::new();
    for size in [512 * 1024, 1024 * 1024] {
        let name = format!("bounded-{size}.bin");
        let source = local.path().join(&name);
        std::fs::write(&source, vec![0x5a; size]).unwrap();
        let id = enqueue_upload(&fixture, session, &transfers, &source, &name);
        assert_eq!(
            finished(&transfers, id).await.state,
            TransferState::Completed
        );
        let page = fixture.list(session, "/files").await.unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap();
        let context = fixture
            .service
            .transfer_context(
                "main",
                "one",
                session,
                TransferTarget::Download { entry: entry.id },
            )
            .unwrap();
        let output = local.path().join(format!("returned-{size}.bin"));
        let destination = capstan_core::transfer::WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&output).unwrap(),
        );
        let selection = transfers
            .grant_destination(&context, Arc::new(destination))
            .unwrap();
        let id = transfers.enqueue(context, selection.token).unwrap();
        assert_eq!(
            finished(&transfers, id).await.state,
            TransferState::Completed
        );
        assert_eq!(std::fs::read(output).unwrap(), vec![0x5a; size]);
        let (current, peak) = transfers.buffer_usage();
        assert_eq!(current, 0);
        assert_eq!(
            peak,
            2 * (capstan_core::sftp::REQUESTS_PER_FILE + 1) * capstan_core::sftp::REQUEST_BYTES
        );
        peaks.push(peak);
    }
    assert_eq!(peaks[0], peaks[1]);
    assert!(peaks[1] <= capstan_core::sftp::MAX_BUFFER_BYTES);
    // The in-process peer stores its own file bodies; this meters the real
    // transfer service's owned payload, not total fixture/process RSS.
    fixture.close().await;
}
