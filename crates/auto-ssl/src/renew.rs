//! 续期阈值判断。

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// 续期策略：距 `not_after` 不足阈值时需要续期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewalPolicy {
    /// 到期前多久开始续期。默认 30 天。
    pub renew_before: Duration,
}

impl RenewalPolicy {
    /// 默认：到期前 30 天。
    pub const fn default_policy() -> Self {
        Self { renew_before: Duration::from_secs(30 * 24 * 60 * 60) }
    }

    /// 自定义阈值。
    pub const fn renew_before(renew_before: Duration) -> Self {
        Self { renew_before }
    }
}

impl Default for RenewalPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// 若 `now + renew_before >= not_after`（含已过期），则需要续期。
pub fn needs_renewal(not_after: SystemTime, now: SystemTime, policy: &RenewalPolicy) -> bool {
    match not_after.duration_since(now) {
        Ok(remaining) => remaining <= policy.renew_before,
        Err(_) => true,
    }
}

/// 距到期还剩多久；已过期则为 `None`。
pub fn time_remaining(not_after: SystemTime, now: SystemTime) -> Option<Duration> {
    not_after.duration_since(now).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renews_inside_threshold() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let policy = RenewalPolicy::renew_before(Duration::from_secs(10));
        let not_after = now + Duration::from_secs(9);
        assert!(needs_renewal(not_after, now, &policy));
    }

    #[test]
    fn skips_outside_threshold() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let policy = RenewalPolicy::renew_before(Duration::from_secs(10));
        let not_after = now + Duration::from_secs(11);
        assert!(!needs_renewal(not_after, now, &policy));
    }

    #[test]
    fn renews_when_already_expired() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let policy = RenewalPolicy::default();
        let not_after = now - Duration::from_secs(1);
        assert!(needs_renewal(not_after, now, &policy));
        assert!(time_remaining(not_after, now).is_none());
    }
}
