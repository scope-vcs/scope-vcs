use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock, PoisonError},
    time::{Duration, Instant},
};

const RETENTION: Duration = Duration::from_secs(10 * 60);

type Omissions = HashMap<String, (Instant, Vec<String>)>;

fn omissions() -> &'static Mutex<Omissions> {
    static OMISSIONS: OnceLock<Mutex<Omissions>> = OnceLock::new();
    OMISSIONS.get_or_init(Mutex::default)
}

pub(super) fn record_build(read_view: &str, unreadable_requests: Vec<String>) {
    let mut omissions = omissions().lock().unwrap_or_else(PoisonError::into_inner);
    if unreadable_requests.is_empty() {
        omissions.remove(read_view);
        return;
    }
    let now = Instant::now();
    omissions.retain(|_, (recorded, _)| now.duration_since(*recorded) < RETENTION);
    omissions.insert(read_view.to_string(), (now, unreadable_requests));
}

pub(super) fn unreadable_requests(read_view: &str) -> Vec<String> {
    omissions()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(read_view)
        .map(|(_, requests)| requests.clone())
        .unwrap_or_default()
}
