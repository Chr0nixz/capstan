//! A byte-transparent owned TCP relay; faults drop sockets rather than replay protocols.
use super::*;
pub(super) struct Relay {
    pub port: u16,
    stop: CancellationToken,
    connections: Arc<std::sync::Mutex<Vec<CancellationToken>>>,
    tasks: TaskTracker,
}
impl Relay {
    pub async fn start(endpoint: u16) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = CancellationToken::new();
        let tasks = TaskTracker::new();
        let connections = Arc::new(std::sync::Mutex::new(Vec::new()));
        let tracked = tasks.clone();
        let quit = stop.clone();
        let current = connections.clone();
        tasks.spawn(async move {
            loop {
                let accepted=tokio::select!{()=quit.cancelled()=>break,result=listener.accept()=>result};
                let (mut incoming,_)=accepted.unwrap();let cancel=quit.child_token();
                current.lock().unwrap().push(cancel.clone());
                tracked.spawn(async move{
                    let mut outgoing=tokio::net::TcpStream::connect(("127.0.0.1",endpoint)).await.unwrap();
                    tokio::select!{()=cancel.cancelled()=>{},_=tokio::io::copy_bidirectional(&mut incoming,&mut outgoing)=>{}}
                });
            }
        });
        Self {
            port,
            stop,
            connections,
            tasks,
        }
    }
    pub fn break_connections(&self) {
        for cancel in self.connections.lock().unwrap().iter() {
            cancel.cancel();
        }
    }
    pub fn opened(&self) -> usize {
        self.connections.lock().unwrap().len()
    }
    pub async fn close(self) {
        self.stop.cancel();
        self.tasks.close();
        tokio::time::timeout(Duration::from_secs(5), self.tasks.wait())
            .await
            .unwrap();
    }
}
