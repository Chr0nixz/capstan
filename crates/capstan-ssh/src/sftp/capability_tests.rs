//! Negotiated resource boundaries against a deterministic SFTP wire peer.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

async fn read(stream: &mut DuplexStream) -> Option<Vec<u8>> {
    let size = stream.read_u32().await.ok()?;
    assert!(size <= MAX_PACKET_BYTES as u32);
    let mut data = vec![0; size as usize];
    stream.read_exact(&mut data).await.ok()?;
    Some(data)
}
async fn write(stream: &mut DuplexStream, data: &[u8]) {
    stream.write_u32(data.len() as u32).await.unwrap();
    stream.write_all(data).await.unwrap();
}
fn string(packet: &mut Vec<u8>, value: &[u8]) {
    packet.extend_from_slice(&(value.len() as u32).to_be_bytes());
    packet.extend_from_slice(value);
}
async fn fixture(
    version: &'static str,
    limits: Option<[u64; 4]>,
) -> (
    Result<Arc<SftpClient>, ClientError>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
) {
    fixture_with_admission(version, limits, &Admission::default()).await
}
async fn fixture_with_admission(
    version: &'static str,
    limits: Option<[u64; 4]>,
    admission: &Admission,
) -> (
    Result<Arc<SftpClient>, ClientError>,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
) {
    let (client, mut stream) = tokio::io::duplex(64 * 1024);
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    let server = tokio::spawn(async move {
        assert_eq!(read(&mut stream).await.unwrap(), [1, 0, 0, 0, 3]);
        let mut response = vec![2, 0, 0, 0, 3];
        for name in ["posix-rename@openssh.com", "fsync@openssh.com"] {
            string(&mut response, name.as_bytes());
            string(&mut response, version.as_bytes());
        }
        if limits.is_some() {
            string(&mut response, b"limits@openssh.com");
            string(&mut response, b"1");
        }
        write(&mut stream, &response).await;
        while let Some(request) = read(&mut stream).await {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut reply = vec![match request[0] {
                200 => 201,
                3 => 102,
                4 => 101,
                other => panic!("unexpected capability request {other}"),
            }];
            reply.extend_from_slice(&request[1..5]);
            match request[0] {
                200 => {
                    for limit in limits.unwrap() {
                        reply.extend_from_slice(&limit.to_be_bytes());
                    }
                }
                3 => string(&mut reply, b"owned-handle"),
                4 => {
                    reply.extend_from_slice(&0_u32.to_be_bytes());
                    string(&mut reply, b"");
                    string(&mut reply, b"");
                }
                _ => unreachable!(),
            }
            write(&mut stream, &reply).await;
        }
    });
    let client =
        SftpClient::connect_with_admission(client, &CancellationToken::new(), admission).await;
    (client, server, requests)
}

#[tokio::test]
async fn shared_handle_admission_keeps_each_subsystems_smaller_advertised_limit() {
    let admission = Admission::default();
    let (one, first_server, _) = fixture_with_admission("1", Some([0, 0, 0, 1]), &admission).await;
    let (two, second_server, _) = fixture_with_admission("1", Some([0, 0, 0, 1]), &admission).await;
    let (one, two) = (one.unwrap(), two.unwrap());
    let cancel = CancellationToken::new();
    let first = one.open_file("/first", true, &cancel).await.unwrap();
    let second = two.open_file("/second", true, &cancel).await.unwrap();
    for client in [&one, &two] {
        assert!(matches!(
            client.open_file("/overflow", true, &cancel).await,
            Err(ClientError::ResourceLimit)
        ));
    }
    assert_eq!(admission.handles.available_permits(), HANDLES - 2);
    first.close(&cancel).await.unwrap();
    let replacement = one.open_file("/replacement", true, &cancel).await.unwrap();
    replacement.close(&cancel).await.unwrap();
    second.close(&cancel).await.unwrap();
    assert_eq!(admission.handles.available_permits(), HANDLES);
    one.close().await;
    two.close().await;
    join(first_server).await;
    join(second_server).await;
}
async fn join(server: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn only_supported_extension_versions_authorize_mutations() {
    for version in ["1", "2", "01", ""] {
        let (client, server, requests) = fixture(version, None).await;
        let client = client.unwrap();
        assert_eq!(client.capabilities().atomic_replace, version == "1");
        assert_eq!(client.capabilities().fsync, version == "1");
        if version != "1" {
            assert_eq!(
                client
                    .atomic_replace("/source", "/target", &CancellationToken::new())
                    .await,
                Err(ClientError::Unsupported)
            );
        }
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        client.close().await;
        join(server).await;
    }
}

#[tokio::test]
async fn advertised_packet_read_and_write_limits_bound_every_payload() {
    for (limits, expected) in [
        ([0, 0, 0, 0], Some(BLOCK)),
        ([1030, 0, 0, 0], Some(6)),
        ([0, 2048, 1024, 0], Some(1024)),
        ([u64::MAX, u64::MAX, u64::MAX, u64::MAX], Some(BLOCK)),
        ([512, 0, 0, 0], None),
    ] {
        let (client, server, requests) = fixture("1", Some(limits)).await;
        if let Some(expected) = expected {
            let client = client.unwrap();
            assert_eq!(client.capabilities().request_bytes, expected);
            client.close().await;
        } else {
            assert!(matches!(client, Err(ClientError::Unavailable)));
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        join(server).await;
    }
}

#[tokio::test]
async fn advertised_handle_limit_rejects_locally_and_recovers_after_close() {
    let (client, server, requests) = fixture("1", Some([0, 0, 0, 1])).await;
    let client = client.unwrap();
    let cancel = CancellationToken::new();
    let first = client.open_file("/first", true, &cancel).await.unwrap();
    assert!(matches!(
        client.open_file("/second", true, &cancel).await,
        Err(ClientError::ResourceLimit)
    ));
    assert!(!client.is_closed());
    assert_eq!(
        requests.load(Ordering::SeqCst),
        2,
        "limits plus exactly one OPEN"
    );
    first.close(&cancel).await.unwrap();
    let second = client.open_file("/second", true, &cancel).await.unwrap();
    second.close(&cancel).await.unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 5);
    client.close().await;
    join(server).await;
}
