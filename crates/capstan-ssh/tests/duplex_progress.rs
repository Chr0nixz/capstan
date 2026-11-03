//! A bounded in-process SSH stream reproduces reciprocal socket backpressure.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use russh::{client, server, Channel, ChannelMsg};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

const BYTES: usize = 8 * 1024 * 1024;

struct Client;
impl client::Handler for Client {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _: &russh::keys::PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct Server {
    start: Arc<Notify>,
    tasks: TaskTracker,
    channels: usize,
    received: Arc<AtomicUsize>,
}
impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let index = self.channels;
        self.channels += 1;
        let start = self.start.clone();
        let received = self.received.clone();
        self.tasks.spawn(async move {
            let (mut reader, writer) = channel.split();
            if index == 0 {
                start.notified().await;
                let send = async {
                    for _ in 0..BYTES / (32 * 1024) {
                        writer.data_bytes(vec![0x5a; 32 * 1024]).await.unwrap();
                    }
                    writer.eof().await.unwrap();
                };
                let receive = async {
                    while let Some(message) = reader.wait().await {
                        match message {
                            ChannelMsg::Data { data } => {
                                assert!(data.iter().all(|byte| *byte == 0xa5));
                                received.fetch_add(data.len(), Ordering::SeqCst);
                            }
                            ChannelMsg::Eof => break,
                            _ => {}
                        }
                    }
                };
                tokio::join!(send, receive);
            } else {
                while let Some(message) = reader.wait().await {
                    match message {
                        ChannelMsg::Data { data } => writer.data_bytes(data).await.unwrap(),
                        ChannelMsg::Eof | ChannelMsg::Close => break,
                        _ => {}
                    }
                }
            }
        });
        Ok(true)
    }
}

async fn connect(
    handler: Server,
    rekey: bool,
) -> (
    client::Handle<Client>,
    tokio::task::JoinHandle<Result<(), russh::Error>>,
) {
    let private =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::ssh_key::Algorithm::Ed25519)
            .unwrap();
    let server_config = server::Config {
        keys: vec![private],
        window_size: 2 * 1024 * 1024,
        maximum_packet_size: 32 * 1024,
        ..server::Config::default()
    };
    let mut client_config = client::Config {
        window_size: 2 * 1024 * 1024,
        maximum_packet_size: 32 * 1024,
        ..client::Config::default()
    };
    if rekey {
        client_config.limits = russh::Limits::new(256 * 1024, 256 * 1024, Duration::from_hours(1));
    }
    // Smaller than one SSH packet: each direction must actually wait for its
    // peer to read. No sleeps, network timing or external daemon are involved.
    let (client_stream, server_stream) = tokio::io::duplex(1024);
    let server = tokio::spawn(async move {
        server::run_stream(Arc::new(server_config), server_stream, handler)
            .await
            .unwrap()
            .await
    });
    let mut connection = client::connect_stream(Arc::new(client_config), client_stream, Client)
        .await
        .unwrap();
    assert!(connection
        .authenticate_password("fixture", "synthetic")
        .await
        .unwrap()
        .success());
    (connection, server)
}

async fn exercise(rekey: bool) {
    let start = Arc::new(Notify::new());
    let received = Arc::new(AtomicUsize::new(0));
    let tasks = TaskTracker::new();
    let (connection, server) = connect(
        Server {
            start: start.clone(),
            tasks: tasks.clone(),
            channels: 0,
            received: received.clone(),
        },
        rekey,
    )
    .await;
    let bulk = connection.channel_open_session().await.unwrap();
    let mut sibling = connection
        .channel_open_session()
        .await
        .unwrap()
        .into_stream();
    let (mut reader, writer) = bulk.split();
    start.notify_one();
    let send = async {
        for _ in 0..BYTES / (32 * 1024) {
            writer.data_bytes(vec![0xa5; 32 * 1024]).await.unwrap();
        }
        writer.eof().await.unwrap();
    };
    let receive = async {
        let mut total = 0;
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    assert!(data.iter().all(|byte| *byte == 0x5a));
                    total += data.len();
                }
                ChannelMsg::Eof => break,
                _ => {}
            }
        }
        assert_eq!(total, BYTES);
    };
    let echo = async {
        use tokio::io::AsyncWriteExt;
        for _ in 0..8 {
            sibling.write_all(b"sibling").await.unwrap();
            let mut reply = [0; 7];
            sibling.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply, b"sibling");
        }
        sibling.shutdown().await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(send, receive, echo);
        tasks.close();
        tasks.wait().await;
    })
    .await
    .expect("SSH must read while an outbound socket write is blocked");
    assert_eq!(received.load(Ordering::SeqCst), BYTES);
    connection
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await
        .unwrap();
    drop(connection);
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn reciprocal_socket_backpressure_preserves_bulk_data_and_sibling_input() {
    exercise(false).await;
}

#[tokio::test]
async fn reciprocal_socket_backpressure_preserves_rekey_ordering() {
    exercise(true).await;
}
