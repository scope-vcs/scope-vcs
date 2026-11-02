use std::time::Duration;

pub const REQUEST_POOL_PACK_LIMIT: usize = 16;

pub const BACKFILL_CADENCE: Duration = Duration::from_secs(30);
pub const BACKFILL_IDLE_CADENCE: Duration = Duration::from_secs(3600);
pub const BACKFILL_LEASE: Duration = Duration::from_secs(600);
pub const BACKFILL_FAILURE_RETRY: Duration = Duration::from_secs(86400);

pub struct PoolRestoreFacts {
    pub head_present: bool,
    pub old_head_present: bool,
    pub connected: bool,
    pub within_reader_limits: bool,
}

#[derive(Debug)]
pub struct VerifiedPoolRestore(());

impl VerifiedPoolRestore {
    pub fn maximum_request_pack_count(&self, verified_pack_count: usize) -> usize {
        verified_pack_count.max(REQUEST_POOL_PACK_LIMIT)
    }
}

impl PoolRestoreFacts {
    pub fn verify(self) -> Result<VerifiedPoolRestore, &'static str> {
        if !self.within_reader_limits {
            return Err("pool restore exceeds the request reader's pack limit");
        }
        if !self.head_present {
            return Err("revision head could not be installed using only pool packs and main");
        }
        if !self.old_head_present {
            return Err("revision old head could not be installed using only pool packs and main");
        }
        if !self.connected {
            return Err("revision history is not connected using only pool packs and main");
        }
        Ok(VerifiedPoolRestore(()))
    }
}

pub fn backfill_failure_reason(reason: &str) -> String {
    let reason = reason
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(2000)
        .collect::<String>();
    if reason.trim().is_empty() {
        "pool restore verification failed".into()
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_requires_readable_connected_heads_within_reader_limits() {
        for missing in 0..4 {
            let facts = PoolRestoreFacts {
                head_present: missing != 0,
                old_head_present: missing != 1,
                connected: missing != 2,
                within_reader_limits: missing != 3,
            };
            assert!(facts.verify().is_err());
        }
        assert!(
            PoolRestoreFacts {
                head_present: true,
                old_head_present: true,
                connected: true,
                within_reader_limits: true,
            }
            .verify()
            .is_ok()
        );
        assert_eq!(
            backfill_failure_reason("\0\t"),
            "pool restore verification failed"
        );
        assert_eq!(backfill_failure_reason("bad\0pack"), "badpack");
    }
}
