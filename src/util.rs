use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[inline(always)]
pub(crate) fn now_unix_time() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock may have gone backward")
}
