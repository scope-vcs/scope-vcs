use std::{future::Future, time::Duration};

#[derive(Debug)]
pub(crate) enum HeartbeatError {
    LeaseLost,
    Database(anyhow::Error),
}

pub(crate) async fn supervise_lease<T, F, R, RFut>(
    future: F,
    lease_duration: Duration,
    mut renew: R,
) -> Result<T, HeartbeatError>
where
    F: Future<Output = T>,
    R: FnMut() -> RFut,
    RFut: Future<Output = anyhow::Result<bool>>,
{
    tokio::pin!(future);
    let mut heartbeat = tokio::time::interval(heartbeat_interval(lease_duration));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = heartbeat.tick() => {
                let renewal = renew();
                tokio::pin!(renewal);
                let renewed = tokio::select! {
                    result = &mut future => return Ok(result),
                    renewed = &mut renewal => renewed,
                };
                match renewed {
                    Ok(true) => {}
                    Ok(false) => return Err(HeartbeatError::LeaseLost),
                    Err(error) => return Err(HeartbeatError::Database(error)),
                }
            }
        }
    }
}

fn heartbeat_interval(lease_duration: Duration) -> Duration {
    (lease_duration / 3).max(Duration::from_millis(10))
}
