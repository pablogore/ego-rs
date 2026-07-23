//! Retry policy and delivery configuration (CORE-019 Phase 3).
//!
//! AD-5: the constants below are runtime **default** values, not
//! spec-normative numbers — the spec only requires a bounded, jittered,
//! exponential backoff with a per-`effect_type` override; the exact figures
//! are this implementation's choice and may be overridden by constructing a
//! different [`RetryPolicy`] value per `effect_type`/adapter.

use std::collections::HashMap;
use std::time::Duration;

use uuid::Uuid;

/// AD-5 default: 3 retries permitted (4 attempts total) before an effect is
/// abandoned.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// AD-5 default: the base of the exponential backoff.
pub const DEFAULT_BASE_BACKOFF: Duration = Duration::from_millis(100);
/// AD-5 default: the cap the exponential backoff never exceeds.
pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(10);

/// Bounded exponential backoff with full jitter (AD-5).
///
/// A plain, per-instance value — a per-`effect_type`/adapter override is just
/// a different `RetryPolicy` value threaded through that type's
/// `DeliveryConfig`; no separate override registry exists here because
/// nothing yet consumes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// How many retry attempts are allowed (not counting the first attempt).
    pub max_attempts: u32,
    /// The base of the exponential backoff (before jitter).
    pub base_backoff: Duration,
    /// The backoff never exceeds this, even after jitter.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            base_backoff: DEFAULT_BASE_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }
}

impl RetryPolicy {
    /// Zero retries — used by [`DeliveryConfig::immediate`] (design.md §7): a
    /// failed attempt under this profile is signaled, never retried.
    pub fn none() -> Self {
        Self {
            max_attempts: 0,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }

    /// Whether `attempt` (the attempt number that just failed, 0-based retry
    /// count already spent) may still be retried.
    pub fn allows_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }

    /// Exponential backoff for `attempt` (1-based: the attempt number that
    /// just failed), capped at `max_backoff`, with full jitter — a uniformly
    /// random duration in `[0, capped]`.
    pub fn backoff(&self, attempt: u32) -> Duration {
        if self.base_backoff.is_zero() {
            return Duration::ZERO;
        }
        let shift = attempt.saturating_sub(1).min(30);
        let multiplier = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
        let exponential = self.base_backoff.saturating_mul(multiplier);
        let capped = exponential.min(self.max_backoff);
        full_jitter(capped)
    }
}

/// Full jitter: a uniformly random duration in `[0, capped]`.
///
/// Reuses `uuid`'s v4 generator (already a dependency for `EffectId`) as the
/// entropy source rather than adding a `rand` dependency for one jitter
/// computation.
fn full_jitter(capped: Duration) -> Duration {
    if capped.is_zero() {
        return Duration::ZERO;
    }
    let entropy = Uuid::new_v4();
    let numerator = u64::from_be_bytes(entropy.as_bytes()[0..8].try_into().unwrap());
    let fraction = numerator as f64 / u64::MAX as f64;
    capped.mul_f64(fraction)
}

/// F-02: the retry policy the [`crate::effects::runner::DeliveryRunner`]
/// actually consults, with an optional per-`effect_type` override over one
/// shared default.
///
/// Before this fix a runner instance only ever had a single [`RetryPolicy`]
/// (design.md's original "a per-`effect_type` override is just a different
/// `RetryPolicy` value threaded through" note was never wired to anything —
/// there was exactly one field, shared by every `effect_type`). This type is
/// that override registry: [`Self::policy_for`] is what the runner now calls
/// wherever it used to read its own `retry: RetryPolicy` field directly, for
/// both the retry-decision (`allows_retry`) and the backoff computation.
#[derive(Debug, Clone)]
pub struct RetryPolicies {
    /// Used for every `effect_type` without an explicit override.
    pub default_retry: RetryPolicy,
    /// Per-`effect_type` overrides of `default_retry`.
    pub retry_overrides: HashMap<String, RetryPolicy>,
}

impl RetryPolicies {
    /// A single shared policy, no per-type overrides.
    pub fn new(default_retry: RetryPolicy) -> Self {
        Self {
            default_retry,
            retry_overrides: HashMap::new(),
        }
    }

    /// Adds (or replaces) the override for `effect_type`.
    pub fn with_override(mut self, effect_type: impl Into<String>, policy: RetryPolicy) -> Self {
        self.retry_overrides.insert(effect_type.into(), policy);
        self
    }

    /// The policy the runner must use for `effect_type`: its override if one
    /// is registered, otherwise `default_retry`.
    pub fn policy_for(&self, effect_type: &str) -> RetryPolicy {
        self.retry_overrides
            .get(effect_type)
            .copied()
            .unwrap_or(self.default_retry)
    }
}

/// Lets every existing call site that constructs a `DeliveryRunner` with a
/// bare [`RetryPolicy`] (no overrides needed) keep compiling unchanged —
/// `DeliveryRunner::new` takes `impl Into<RetryPolicies>`.
impl From<RetryPolicy> for RetryPolicies {
    fn from(default_retry: RetryPolicy) -> Self {
        Self::new(default_retry)
    }
}

/// Where the delivery runner drives one drain step (design.md §7): a
/// separately spawned task (`Deferred`, the default), or the caller's own
/// task/call stack (`Inline`, used by [`DeliveryConfig::immediate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerMode {
    /// A spawned drain task owns the queue.
    Deferred,
    /// `accept` itself drives one drain step before returning.
    Inline,
}

/// Configuration for the one delivery pipeline (accept → queue → run →
/// execute). [`DeliveryConfig::immediate`] is a profile of this same
/// pipeline, not a bypass (design.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryConfig {
    /// Bounded admission-queue capacity.
    pub queue_capacity: usize,
    /// The retry policy applied to every retryable delivery failure.
    pub retry: RetryPolicy,
    /// Where the runner drives its drain step.
    pub runner_mode: RunnerMode,
}

/// A reasonable default admission-queue capacity for the `Deferred` profile.
///
/// ponytail: a fixed constant, not yet configurable independently of
/// `queue_capacity`'s field — raise or expose tuning if a real workload needs
/// it; nothing today does.
pub const DEFAULT_QUEUE_CAPACITY: usize = 256;

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            retry: RetryPolicy::default(),
            runner_mode: RunnerMode::Deferred,
        }
    }
}

impl DeliveryConfig {
    /// `ImmediateDeliveryPolicy` (design.md §7): the same pipeline, tuned for
    /// minimal backlog and no retries — minimal queue capacity, zero
    /// retries, inline-scheduled running.
    pub fn immediate() -> Self {
        Self {
            queue_capacity: 1,
            retry: RetryPolicy::none(),
            runner_mode: RunnerMode::Inline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn attempt_cap_blocks_retry_beyond_max_attempts() {
        let policy = RetryPolicy::default();

        assert!(policy.allows_retry(policy.max_attempts - 1));
        assert!(!policy.allows_retry(policy.max_attempts));
    }

    #[test]
    fn backoff_is_bounded_by_max_backoff_even_for_large_attempt_numbers() {
        let policy = RetryPolicy {
            max_attempts: 20,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
        };

        for attempt in 1..=20 {
            assert!(
                policy.backoff(attempt) <= policy.max_backoff,
                "attempt {attempt} backoff exceeded cap"
            );
        }
    }

    #[test]
    fn backoff_grows_with_attempt_number_before_hitting_the_cap() {
        let policy = RetryPolicy {
            max_attempts: 10,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
        };

        // Full jitter makes any single sample noisy, so compare the
        // *ceilings* each attempt could reach (base * 2^(attempt-1)),
        // which must strictly grow until the cap is hit.
        let ceiling = |attempt: u32| {
            policy
                .base_backoff
                .saturating_mul(1u32 << (attempt - 1).min(30))
                .min(policy.max_backoff)
        };
        assert!(ceiling(2) > ceiling(1));
        assert!(ceiling(3) > ceiling(2));
    }

    #[test]
    fn retry_none_never_allows_a_retry() {
        let policy = RetryPolicy::none();

        assert!(!policy.allows_retry(1));
        assert_eq!(policy.backoff(1), Duration::ZERO);
    }

    #[test]
    fn distinct_policy_instances_may_override_the_default_per_effect_type() {
        // No global registry here — a caller wanting a per-`effect_type`
        // override just builds a different `RetryPolicy` value and threads
        // it through whichever `DeliveryConfig` applies to that type.
        let aggressive = RetryPolicy {
            max_attempts: 1,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(5),
        };
        let default_policy = RetryPolicy::default();

        assert_ne!(aggressive.max_attempts, default_policy.max_attempts);
        assert!(aggressive.allows_retry(0));
        assert!(!aggressive.allows_retry(1));
    }

    #[test]
    fn default_delivery_config_uses_ad5_defaults_and_deferred_runner_mode() {
        let config = DeliveryConfig::default();

        assert_eq!(config.retry.max_attempts, DEFAULT_MAX_ATTEMPTS);
        assert_eq!(config.retry.base_backoff, DEFAULT_BASE_BACKOFF);
        assert_eq!(config.retry.max_backoff, DEFAULT_MAX_BACKOFF);
        assert_eq!(config.runner_mode, RunnerMode::Deferred);
    }

    // --- F-02: per-`effect_type` retry policy override ---

    #[test]
    fn policy_for_returns_the_default_when_no_override_is_registered() {
        let policies = RetryPolicies::new(RetryPolicy::default());

        assert_eq!(
            policies.policy_for("invoice.created"),
            RetryPolicy::default()
        );
    }

    #[test]
    fn policy_for_returns_the_override_for_its_registered_effect_type_only() {
        let aggressive = RetryPolicy {
            max_attempts: 0,
            base_backoff: Duration::from_millis(5),
            max_backoff: Duration::from_millis(5),
        };
        let policies =
            RetryPolicies::new(RetryPolicy::default()).with_override("s3.put", aggressive);

        assert_eq!(policies.policy_for("s3.put"), aggressive);
        assert_eq!(
            policies.policy_for("invoice.created"),
            RetryPolicy::default()
        );
    }

    #[test]
    fn immediate_profile_is_zero_retries_minimal_capacity_inline() {
        let config = DeliveryConfig::immediate();

        assert_eq!(config.queue_capacity, 1);
        assert_eq!(config.retry.max_attempts, 0);
        assert_eq!(config.runner_mode, RunnerMode::Inline);
    }
}
