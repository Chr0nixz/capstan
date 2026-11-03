use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, Notify},
};

const SIZE: usize = 70 * BLOCK + 17;
fn body(offset: usize, count: usize) -> Vec<u8> {
    (offset..offset + count).map(|n| (n % 251) as u8).collect()
}
fn string(packet: &mut Vec<u8>, bytes: &[u8]) {
    packet.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    packet.extend_from_slice(bytes);
}
fn take_string(packet: &[u8], cursor: &mut usize) -> Vec<u8> {
    let count = u32::from_be_bytes(packet[*cursor..*cursor + 4].try_into().unwrap()) as usize;
    *cursor += 4;
    let bytes = packet[*cursor..*cursor + count].to_vec();
    *cursor += count;
    bytes
}
async fn packet(read: &mut (impl AsyncRead + Unpin)) -> Option<Vec<u8>> {
    let length = read.read_u32().await.ok()? as usize;
    assert!(length <= MAX_PACKET_BYTES);
    let mut bytes = vec![0; length];
    read.read_exact(&mut bytes).await.ok()?;
    Some(bytes)
}
async fn send(write: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) {
    write.write_u32(bytes.len() as u32).await.unwrap();
    write.write_all(bytes).await.unwrap();
}
fn reply(kind: u8, id: &[u8]) -> Vec<u8> {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(id);
    bytes
}
fn status(id: &[u8], denied: bool) -> Vec<u8> {
    let mut bytes = reply(101, id);
    bytes.extend_from_slice(&(if denied { 3_u32 } else { 0 }).to_be_bytes());
    string(&mut bytes, b"");
    string(&mut bytes, b"");
    bytes
}
#[derive(Default)]
struct Metrics {
    held_writes: AtomicUsize,
    held_stats: AtomicUsize,
    peak_read_batch: AtomicUsize,
    gap_requests: AtomicUsize,
}
struct Peer {
    client: Arc<SftpClient>,
    metrics: Arc<Metrics>,
    release: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Peer {
    async fn new(reverse_reads: bool, denied: Option<u64>) -> Self {
        Self::with_admission(reverse_reads, denied, &Admission::default()).await
    }
    async fn with_admission(
        reverse_reads: bool,
        denied: Option<u64>,
        admission: &Admission,
    ) -> Self {
        let (stream, peer) = tokio::io::duplex(64 * 1024);
        let metrics = Arc::new(Metrics::default());
        let counts = metrics.clone();
        let release = Arc::new(Notify::new());
        let released = release.clone();
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(peer);
            let (replies, mut rx) = mpsc::channel::<Vec<u8>>(256);
            let writer = tokio::spawn(async move {
                while let Some(bytes) = rx.recv().await {
                    send(&mut write, &bytes).await;
                }
            });
            let held = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
            let release_held = held.clone();
            let holding = Arc::new(AtomicBool::new(true));
            let release_holding = holding.clone();
            let release_replies = replies.clone();
            let releaser = tokio::spawn(async move {
                released.notified().await;
                release_holding.store(false, Ordering::SeqCst);
                let pending = std::mem::take(&mut *release_held.lock().unwrap());
                for bytes in pending.into_iter().rev() {
                    release_replies.send(bytes).await.unwrap();
                }
            });
            assert_eq!(packet(&mut read).await.unwrap(), [1, 0, 0, 0, 3]);
            replies.send(vec![2, 0, 0, 0, 3]).await.unwrap();
            let mut read_batch = Vec::new();
            let mut reversed = !reverse_reads;
            while let Some(request) = packet(&mut read).await {
                let id = &request[1..5];
                let mut cursor = 5;
                let response = match request[0] {
                    3 => {
                        let handle = take_string(&request, &mut cursor);
                        let mut bytes = reply(102, id);
                        string(&mut bytes, &handle);
                        bytes
                    }
                    4 => status(id, false),
                    6 => {
                        let handle = take_string(&request, &mut cursor);
                        let offset =
                            u64::from_be_bytes(request[cursor..cursor + 8].try_into().unwrap());
                        let bytes = status(id, denied == Some(offset));
                        if handle.starts_with(b"/slow") && holding.load(Ordering::SeqCst) {
                            held.lock().unwrap().push(bytes);
                            counts.held_writes.fetch_add(1, Ordering::SeqCst);
                            continue;
                        }
                        bytes
                    }
                    5 => {
                        let _handle = take_string(&request, &mut cursor);
                        let offset =
                            u64::from_be_bytes(request[cursor..cursor + 8].try_into().unwrap())
                                as usize;
                        let length = u32::from_be_bytes(
                            request[cursor + 8..cursor + 12].try_into().unwrap(),
                        ) as usize;
                        assert!(length <= BLOCK);
                        if offset == BLOCK + 127 && reverse_reads {
                            assert_eq!(length, BLOCK - 127);
                            counts.gap_requests.fetch_add(1, Ordering::SeqCst);
                        }
                        let bytes = if offset >= SIZE {
                            let mut bytes = status(id, false);
                            bytes[5..9].copy_from_slice(&1_u32.to_be_bytes());
                            bytes
                        } else {
                            let count = if reverse_reads && offset == BLOCK {
                                127
                            } else {
                                length.min(SIZE - offset)
                            };
                            let mut bytes = reply(103, id);
                            string(&mut bytes, &body(offset, count));
                            bytes
                        };
                        // Initial read learns the chunk size. The next entire
                        // window must arrive before any reversed replies.
                        if !reversed && offset >= BLOCK {
                            read_batch.push(bytes);
                            counts
                                .peak_read_batch
                                .fetch_max(read_batch.len(), Ordering::SeqCst);
                            if read_batch.len() == FILE_REQUESTS {
                                reversed = true;
                                for bytes in read_batch.drain(..).rev() {
                                    replies.send(bytes).await.unwrap();
                                }
                            }
                            continue;
                        }
                        bytes
                    }
                    7 => {
                        let path = take_string(&request, &mut cursor);
                        let mut bytes = reply(105, id);
                        bytes.extend_from_slice(&13_u32.to_be_bytes());
                        bytes.extend_from_slice(&(SIZE as u64).to_be_bytes());
                        bytes.extend_from_slice(&0o100600_u32.to_be_bytes());
                        bytes.extend_from_slice(&100_u32.to_be_bytes());
                        bytes.extend_from_slice(&100_u32.to_be_bytes());
                        if path.starts_with(b"/hold") && holding.load(Ordering::SeqCst) {
                            held.lock().unwrap().push(bytes);
                            counts.held_stats.fetch_add(1, Ordering::SeqCst);
                            continue;
                        }
                        bytes
                    }
                    kind => panic!("unexpected bounded-pipeline request {kind}"),
                };
                replies.send(response).await.unwrap();
            }
            releaser.abort();
            let _ = releaser.await;
            drop(replies);
            writer.await.unwrap();
        });
        let client =
            SftpClient::connect_with_admission(stream, &CancellationToken::new(), admission)
                .await
                .unwrap();
        Self {
            client,
            metrics,
            release,
            task,
        }
    }
    async fn file(&self, name: &str, write: bool) -> PipelinedFile {
        self.client
            .open_file(name, write, &CancellationToken::new())
            .await
            .unwrap()
            .pipeline(&CancellationToken::new())
            .await
            .unwrap()
    }
    async fn close(self) {
        self.client.close().await;
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap();
    }
    async fn full_window(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.metrics.held_writes.load(Ordering::SeqCst) != FILE_REQUESTS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn three_subsystems_share_pipeline_and_waiter_admission_without_revoking_siblings_on_close() {
    let admission = Admission::default();
    let one = Peer::with_admission(false, None, &admission).await;
    let two = Peer::with_admission(false, None, &admission).await;
    let three = Peer::with_admission(false, None, &admission).await;
    let slow = one.file("/slow", true).await;
    let mut upload = two.file("/upload", true).await;
    let mut download = three.file("/download", false).await;
    assert_eq!(admission.slots.available_permits(), 16);
    assert_eq!(admission.callers.available_permits(), 16 + WAITING);

    let cancel = CancellationToken::new();
    let mut requests = Vec::new();
    for index in 0..16 + WAITING {
        let client = if index % 2 == 0 {
            two.client.clone()
        } else {
            three.client.clone()
        };
        let cancel = cancel.clone();
        requests.push(tokio::spawn(
            async move { client.lstat("/hold", &cancel).await },
        ));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while admission.callers.available_permits() != 0
            || two.metrics.held_stats.load(Ordering::SeqCst)
                + three.metrics.held_stats.load(Ordering::SeqCst)
                != 16
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        one.client
            .lstat("/overflow", &CancellationToken::new())
            .await,
        Err(ClientError::ResourceLimit)
    );
    cancel.cancel();
    for request in requests {
        assert_eq!(request.await.unwrap(), Err(ClientError::Canceled));
    }
    // The sixteen late replies retain admission after their callers cancel.
    assert_eq!(admission.slots.available_permits(), 0);
    assert_eq!(admission.callers.available_permits(), WAITING);
    drop(slow);
    one.client.close().await;
    one.client.wait_pending().await;
    assert_eq!(admission.slots.available_permits(), FILE_REQUESTS);
    assert!(!two.client.is_closed());
    assert!(!three.client.is_closed());
    two.release.notify_one();
    three.release.notify_one();
    two.client.wait_pending().await;
    three.client.wait_pending().await;
    for _ in 0..FILE_REQUESTS + 1 {
        upload
            .write(&vec![7; BLOCK], &CancellationToken::new())
            .await
            .unwrap();
    }
    upload.drain(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        download
            .read(BLOCK, &CancellationToken::new())
            .await
            .unwrap(),
        body(0, BLOCK)
    );
    upload.close(&CancellationToken::new()).await.unwrap();
    download.close(&CancellationToken::new()).await.unwrap();
    assert_eq!(admission.slots.available_permits(), IN_FLIGHT);
    assert_eq!(admission.callers.available_permits(), IN_FLIGHT + WAITING);
    assert_eq!(admission.handles.available_permits(), HANDLES);
    one.close().await;
    two.close().await;
    three.close().await;
}

#[tokio::test]
async fn remote_handle_admission_is_shared_across_subsystems_and_recovers_after_close() {
    let admission = Admission::default();
    let peers = [
        Peer::with_admission(false, None, &admission).await,
        Peer::with_admission(false, None, &admission).await,
        Peer::with_admission(false, None, &admission).await,
    ];
    let cancel = CancellationToken::new();
    let mut files = Vec::new();
    for index in 0..HANDLES {
        files.push(
            peers[index % 3]
                .client
                .open_file(&format!("/file-{index}"), true, &cancel)
                .await
                .unwrap(),
        );
    }
    for peer in &peers {
        assert!(matches!(
            peer.client.open_file("/overflow", true, &cancel).await,
            Err(ClientError::ResourceLimit)
        ));
    }
    files.pop().unwrap().close(&cancel).await.unwrap();
    let replacement = peers[1]
        .client
        .open_file("/replacement", true, &cancel)
        .await
        .unwrap();
    replacement.close(&cancel).await.unwrap();
    for file in files {
        file.close(&cancel).await.unwrap();
    }
    assert_eq!(admission.handles.available_permits(), HANDLES);
    for peer in peers {
        peer.close().await;
    }
}

#[tokio::test]
async fn full_write_window_reports_zero_until_ack_then_only_the_contiguous_prefix() {
    let peer = Peer::new(false, None).await;
    let mut file = peer.file("/slow", true).await;
    let block = vec![0x5a; BLOCK];
    for _ in 0..FILE_REQUESTS {
        assert_eq!(
            file.write(&block, &CancellationToken::new()).await.unwrap(),
            0
        );
    }
    peer.full_window().await;
    assert_eq!(file.confirmed_bytes(), 0);
    {
        let cancel = CancellationToken::new();
        let next = file.write(&block, &cancel);
        tokio::pin!(next);
        tokio::select! { biased; result = &mut next => panic!("unacknowledged window advanced: {result:?}"), () = tokio::task::yield_now() => {} }
        peer.client
            .lstat("/control", &CancellationToken::new())
            .await
            .unwrap();
        peer.release.notify_one();
        assert_eq!(next.await.unwrap(), BLOCK as u64);
    }
    file.drain(&CancellationToken::new()).await.unwrap();
    assert_eq!(file.confirmed_bytes(), ((FILE_REQUESTS + 1) * BLOCK) as u64);
    file.close(&CancellationToken::new()).await.unwrap();
    assert_eq!(peer.client.slots.available_permits(), IN_FLIGHT);
    peer.close().await;
}

#[tokio::test]
async fn reversed_reads_preserve_short_read_gaps_and_converge_before_close() {
    let peer = Peer::new(true, None).await;
    let mut file = peer.file("/read", false).await;
    let mut received = Vec::new();
    loop {
        let bytes = file.read(BLOCK, &CancellationToken::new()).await.unwrap();
        if bytes.is_empty() {
            break;
        }
        received.extend_from_slice(&bytes);
    }
    assert_eq!(received, body(0, SIZE));
    assert_eq!(
        peer.metrics.peak_read_batch.load(Ordering::SeqCst),
        FILE_REQUESTS
    );
    assert_eq!(peer.metrics.gap_requests.load(Ordering::SeqCst), 1);
    file.close(&CancellationToken::new()).await.unwrap();
    assert!(!peer.client.is_closed());
    assert_eq!(peer.client.slots.available_permits(), IN_FLIGHT);
    peer.close().await;
}

#[tokio::test]
async fn a_failed_ack_does_not_confirm_a_hole_and_known_errors_keep_the_subsystem_usable() {
    let peer = Peer::new(false, Some(BLOCK as u64)).await;
    let mut file = peer.file("/slow", true).await;
    for _ in 0..FILE_REQUESTS {
        file.write(&vec![0; BLOCK], &CancellationToken::new())
            .await
            .unwrap();
    }
    peer.full_window().await;
    peer.release.notify_one();
    assert_eq!(
        file.drain(&CancellationToken::new()).await,
        Err(ClientError::PermissionDenied)
    );
    assert_eq!(file.confirmed_bytes(), BLOCK as u64);
    file.close(&CancellationToken::new()).await.unwrap();
    peer.client
        .lstat("/control", &CancellationToken::new())
        .await
        .unwrap();
    assert!(!peer.client.is_closed());
    peer.close().await;
}

#[tokio::test]
async fn three_file_windows_leave_control_headroom_and_siblings_progress_past_a_stall() {
    let peer = Peer::new(false, None).await;
    let mut slow = peer.file("/slow", true).await;
    let mut upload = peer.file("/upload", true).await;
    let mut download = peer.file("/download", false).await;
    assert_eq!(peer.client.slots.available_permits(), 16);
    for _ in 0..FILE_REQUESTS {
        slow.write(&vec![0; BLOCK], &CancellationToken::new())
            .await
            .unwrap();
    }
    peer.full_window().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        peer.client
            .lstat("/control", &CancellationToken::new())
            .await
            .unwrap();
        for _ in 0..FILE_REQUESTS + 1 {
            upload
                .write(&vec![0; BLOCK], &CancellationToken::new())
                .await
                .unwrap();
        }
        upload.drain(&CancellationToken::new()).await.unwrap();
        assert_eq!(
            download
                .read(BLOCK, &CancellationToken::new())
                .await
                .unwrap(),
            body(0, BLOCK)
        );
    })
    .await
    .unwrap();
    assert_eq!(slow.confirmed_bytes(), 0);
    peer.release.notify_one();
    slow.close(&CancellationToken::new()).await.unwrap();
    upload.close(&CancellationToken::new()).await.unwrap();
    download.close(&CancellationToken::new()).await.unwrap();
    assert_eq!(peer.client.slots.available_permits(), IN_FLIGHT);
    peer.close().await;
}

#[tokio::test]
async fn canceling_a_full_pipeline_reclaims_it_without_interrupting_siblings() {
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let peer = Peer::new(false, None).await;
    let mut slow = peer.file("/slow", true).await;
    let mut sibling = peer.file("/sibling", true).await;
    let released = Arc::new(AtomicBool::new(false));
    slow.retain_payload_guard(Box::new(Guard(released.clone())));
    let cancel = CancellationToken::new();
    for _ in 0..FILE_REQUESTS {
        slow.write(&vec![0; BLOCK], &cancel).await.unwrap();
    }
    peer.full_window().await;
    let block = vec![1; BLOCK];
    {
        let write = slow.write(&block, &cancel);
        tokio::pin!(write);
        tokio::select! { biased; result = &mut write => panic!("stalled write completed: {result:?}"), () = tokio::task::yield_now() => {} }
        cancel.cancel();
        assert_eq!(write.await, Err(ClientError::Uncertain));
    }
    // Drop the request futures and their late replies without closing SFTP.
    drop(slow);
    assert!(!released.load(Ordering::SeqCst));
    assert_eq!(
        peer.client.slots.available_permits(),
        IN_FLIGHT - 2 * FILE_REQUESTS
    );
    assert!(!peer.client.is_closed());
    for _ in 0..FILE_REQUESTS + 1 {
        sibling
            .write(&vec![2; BLOCK], &CancellationToken::new())
            .await
            .unwrap();
    }
    sibling.drain(&CancellationToken::new()).await.unwrap();
    peer.client
        .lstat("/control", &CancellationToken::new())
        .await
        .unwrap();
    peer.release.notify_one();
    sibling.close(&CancellationToken::new()).await.unwrap();
    peer.client.wait_pending().await;
    assert!(released.load(Ordering::SeqCst));
    assert!(!peer.client.is_closed());
    assert_eq!(peer.client.slots.available_permits(), IN_FLIGHT);
    peer.close().await;
}
