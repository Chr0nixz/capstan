//! An emulated slow access link in front of the in-process server.
//!
//! Each direction is a FIFO bottleneck with a fixed rate followed by a fixed
//! one-way delay. The bottleneck queue is unbounded, standing in for an
//! oversized router buffer: whatever the client sends ahead of a keystroke
//! delays that keystroke by its size divided by the rate.
use super::*;
use tokio::{
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    time::Instant,
};

#[derive(Clone, Copy)]
pub(super) struct Link {
    pub(super) bytes_per_second: u64,
    pub(super) one_way: Duration,
}

/// Relays every accepted connection to `endpoint` through `link`. The relay
/// stops with `cancel` and is tracked by `tasks`.
pub(super) async fn start(
    endpoint: u16,
    link: Link,
    tasks: &TaskTracker,
    cancel: CancellationToken,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let tracked = tasks.clone();
    tasks.spawn(async move {
        loop {
            let accepted = tokio::select! {
                () = cancel.cancelled() => break,
                result = listener.accept() => result,
            };
            let (incoming, _) = accepted.unwrap();
            let outgoing = TcpStream::connect(("127.0.0.1", endpoint)).await.unwrap();
            incoming.set_nodelay(true).unwrap();
            outgoing.set_nodelay(true).unwrap();
            let (client_read, client_write) = incoming.into_split();
            let (server_read, server_write) = outgoing.into_split();
            tracked.spawn(direction(client_read, server_write, link, cancel.clone()));
            tracked.spawn(direction(server_read, client_write, link, cancel.clone()));
        }
    });
    port
}

async fn direction(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    link: Link,
    cancel: CancellationToken,
) {
    let (queue, mut queued) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let read = async move {
        let mut free_at = Instant::now();
        let mut buffer = vec![0; 16 * 1024];
        while let Ok(count) = from.read(&mut buffer).await {
            if count == 0 {
                break;
            }
            let serialization =
                Duration::from_secs_f64(count as f64 / link.bytes_per_second as f64);
            free_at = free_at.max(Instant::now()) + serialization;
            if queue
                .send((free_at + link.one_way, buffer[..count].to_vec()))
                .is_err()
            {
                break;
            }
        }
    };
    let write = async move {
        while let Some((deliver, bytes)) = queued.recv().await {
            tokio::time::sleep_until(deliver).await;
            if to.write_all(&bytes).await.is_err() {
                return;
            }
        }
        let _ = to.shutdown().await;
    };
    tokio::select! {
        () = cancel.cancelled() => {}
        _ = async { tokio::join!(read, write) } => {}
    }
}
