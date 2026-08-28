use tokio::sync::watch;

/// Waits until shutdown is requested or all request senders are dropped.
pub async fn requested(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() || receiver.changed().await.is_err() {
            return;
        }
    }
}
