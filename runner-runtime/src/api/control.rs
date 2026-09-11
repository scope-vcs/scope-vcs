use super::*;

impl RuntimeClient {
    pub fn claim(&self, bootstrap_token: &str) -> anyhow::Result<ClaimRuntimeResponse> {
        let response = self
            .client
            .post(self.url("claim"))
            .timeout(CONTROL_REQUEST_TIMEOUT)
            .bearer_auth(bootstrap_token)
            .send()
            .context("claim cloud run attempt")?;
        let response: ClaimRuntimeResponse = json(response, "claim cloud run attempt")?;
        *self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned") = Some(response.attempt_token.clone());
        *self
            .cache_access
            .lock()
            .expect("cache access mutex poisoned") = Some(CacheAccess {
            endpoint: response.cache_endpoint.clone(),
            grant: response.cache_grant.clone(),
        });
        Ok(response)
    }

    pub fn start_step(&self, step: u32) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json(
            &format!("steps/{step}/start"),
            &serde_json::json!({}),
            "start step",
        )
    }

    pub fn heartbeat(&self) -> anyhow::Result<AttemptStatusResponse> {
        #[cfg(test)]
        if let Some(started) = &self.heartbeat_started {
            let _ = started.send(());
        }
        let _heartbeat = self
            .heartbeat_lock
            .lock()
            .expect("heartbeat mutex poisoned");
        let cache_keys = self
            .cache_keys
            .lock()
            .expect("cache keys mutex poisoned")
            .clone();
        let response: AttemptHeartbeatResponse = self.post_json(
            "heartbeat",
            &AttemptHeartbeatRequest { cache_keys },
            "heartbeat attempt",
        )?;
        let mut access = self
            .cache_access
            .lock()
            .expect("cache access mutex poisoned");
        let access = access
            .as_mut()
            .context("cache access is unavailable before attempt claim")?;
        access.grant = response.cache_grant;
        Ok(response.status)
    }

    pub fn authorize_cache_keys(
        &self,
        cache_keys: Vec<AttemptCacheKeyMaterial>,
    ) -> anyhow::Result<()> {
        *self.cache_keys.lock().expect("cache keys mutex poisoned") = cache_keys;
        self.heartbeat()?;
        Ok(())
    }
}
