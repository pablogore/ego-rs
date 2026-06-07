use tokio::sync::{Mutex, watch};

pub struct SharedActivation {
    pub lock: Mutex<()>,
    pub result_tx: watch::Sender<Option<super::error::EntityError>>,
    pub result_rx: watch::Receiver<Option<super::error::EntityError>>,
}

impl SharedActivation {
    pub fn new() -> Self {
        let (result_tx, result_rx) = watch::channel(None);
        SharedActivation {
            lock: Mutex::new(()),
            result_tx,
            result_rx,
        }
    }
}
