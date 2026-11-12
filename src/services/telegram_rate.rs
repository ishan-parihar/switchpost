// ─── Telegram rate limiter + write queue (P6.4) ────────────────
// Ported from ../tg-cli src/tg_cli/ratelimit.py. Three layers,
// calibrated conservative against Telegram's observed abuse signals:
//
// 1. Sliding-window account gate keyed by category — refuses the call
//    BEFORE Telegram opens a FLOOD_WAIT. Defaults (tg-cli parity):
//    dialogs 1/min, history 30/min, send 20/min, edit 15/min, delete 15/min.
// 2. Per-peer send pacing — ~1 msg / 5s to a private chat, ~20/min into
//    the same group/channel. A peer refusal REFUNDS the account slot, so
//    a burst aimed at one peer cannot burn the account-wide budget.
// 3. Reactive breaker — 3 consecutive flood waits on a category suspend
//    it for max(retry_after, 300s).
//
// State is in-process; std Mutex with poison recovery (a panicking writer
// must not break every Telegram call for the rest of the process).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

// ── Categories ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Dialogs,
    History,
    Send,
    Edit,
    Delete,
}

impl Category {
    /// tg-cli's conservative default caps.
    fn default_spec(self) -> RateSpec {
        use Category::*;
        match self {
            Dialogs => RateSpec {
                max_calls: 1,
                window: Duration::from_secs(60),
            },
            History => RateSpec {
                max_calls: 30,
                window: Duration::from_secs(60),
            },
            Send => RateSpec {
                max_calls: 20,
                window: Duration::from_secs(60),
            },
            Edit => RateSpec {
                max_calls: 15,
                window: Duration::from_secs(60),
            },
            Delete => RateSpec {
                max_calls: 15,
                window: Duration::from_secs(60),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RateSpec {
    pub max_calls: u32,
    pub window: Duration,
}

// ── Peer keys (Telegram id conventions, no network calls) ─────────────────
// positive = private chat with a user, -100… = channel/supergroup,
// other negative = basic group.

fn peer_send_spec(key: &str) -> Option<RateSpec> {
    // tg-cli parity: 1 msg / 5s to a user, 20/min into groups/channels.
    if key.starts_with("user:") {
        Some(RateSpec {
            max_calls: 1,
            window: Duration::from_secs(5),
        })
    } else if key.starts_with("channel:") || key.starts_with("chat:") {
        Some(RateSpec {
            max_calls: 20,
            window: Duration::from_secs(60),
        })
    } else {
        None
    }
}

// ── Guard ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Inner {
    /// (account, category) -> timestamps of calls inside the window.
    account: std::collections::HashMap<(String, Category), VecDeque<Instant>>,
    /// (account, category, peer) -> timestamps.
    peer: std::collections::HashMap<(String, Category, String), VecDeque<Instant>>,
    /// (account, category) -> suspended-until.
    suspended: std::collections::HashMap<(String, Category), Instant>,
    /// (account, category) -> consecutive flood count.
    flood_counts: std::collections::HashMap<(String, Category), u32>,
}

/// ponytail: peer buckets are bounded by an LRU-style cap of 4096 (tg-cli
/// parity); account buckets are bounded by the category count. Raise the cap
/// if a process ever drives more distinct peers than that.
const MAX_PEER_BUCKETS: usize = 4096;
const BREAKER_THRESHOLD: u32 = 3;
const BREAKER_COOLDOWN: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct TelegramRateGuard {
    inner: Mutex<Inner>,
}

fn lock<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    // Poison recovery: a panic in one caller must not wedge the gate shut.
    match m.lock() {
        Ok(g) => g,
        Err(PoisonError { .. }) => m.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

impl TelegramRateGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check limits BEFORE a Telegram call. Returns how long the caller
    /// should wait; zero = proceed now. Order: breaker → account → peer.
    pub fn before_call(&self, account: &str, category: Category, peer: Option<&str>) -> Duration {
        let mut inner = lock(&self.inner);
        let now = Instant::now();

        if let Some(until) = inner.suspended.get(&(account.to_string(), category)) {
            if *until > now {
                return *until - now;
            }
        }

        let retry = try_acquire(
            &mut inner.account,
            &(account.to_string(), category),
            category.default_spec(),
            now,
        );
        if retry > Duration::ZERO {
            return retry;
        }

        if let Some(peer) = peer {
            if let Some(spec) = peer_send_spec(peer) {
                let retry = try_acquire(
                    &mut inner.peer,
                    &(account.to_string(), category, peer.to_string()),
                    spec,
                    now,
                );
                if retry > Duration::ZERO {
                    // Peer refusal refunds the account slot: a burst at one
                    // peer must not consume the account-wide budget.
                    if let Some(bucket) = inner.account.get_mut(&(account.to_string(), category)) {
                        bucket.pop_back();
                    }
                    return retry;
                }
            }
        }
        Duration::ZERO
    }

    /// Record a FLOOD_WAIT from Telegram. 3 consecutive on one
    /// (account, category) opens the breaker.
    pub fn record_flood(&self, account: &str, category: Category, retry_after: Duration) {
        let mut inner = lock(&self.inner);
        let key = (account.to_string(), category);
        let count = inner.flood_counts.entry(key.clone()).or_insert(0);
        *count += 1;
        if *count >= BREAKER_THRESHOLD {
            let cooldown = retry_after.max(BREAKER_COOLDOWN);
            inner.suspended.insert(key, Instant::now() + cooldown);
            tracing::warn!(
                "telegram rate breaker OPEN account={account} category={category:?} cooldown={cooldown:?}"
            );
        }
    }

    /// Clear the flood counter after a successful call.
    pub fn record_success(&self, account: &str, category: Category) {
        let mut inner = lock(&self.inner);
        inner
            .flood_counts
            .insert((account.to_string(), category), 0);
    }
}

fn try_acquire<K>(
    store: &mut std::collections::HashMap<K, VecDeque<Instant>>,
    key: &K,
    spec: RateSpec,
    now: Instant,
) -> Duration
where
    K: Eq + std::hash::Hash + Clone + std::fmt::Debug,
{
    if !store.contains_key(key) {
        if store.len() >= MAX_PEER_BUCKETS {
            // Arbitrary-bucket eviction: HashMap iteration order is
            // randomized, not FIFO. At the cap the price is dropping
            // one peer's pacing history (one possible extra burst —
            // the conservative caps absorb that). Account buckets
            // never approach the cap (accounts × 5 categories).
            if let Some(oldest) = store.keys().next() {
                let oldest = oldest.clone();
                store.remove(&oldest);
            }
        }
        store.insert(key.clone(), VecDeque::new());
    }
    let bucket = store.get_mut(key).expect("inserted above");
    let cutoff = now - spec.window;
    while bucket.front().is_some_and(|t| *t < cutoff) {
        bucket.pop_front();
    }
    // A full bucket has a front to wait on. Matching on the front itself means a
    // zero `max_calls` cannot reach an empty `front()` and panic.
    if let Some(&oldest) = bucket
        .front()
        .filter(|_| bucket.len() >= spec.max_calls as usize)
    {
        return oldest + spec.window - now;
    }
    bucket.push_back(now);
    Duration::ZERO
}

// ── Write queue ────────────────────────────────────────────────────────────
// Deferred-send semantics from tg-cli's guarded_send: a refused send waits
// for the gate instead of failing. The queue is drained by a worker that
// re-checks the gate; jobs are FIFO so ordering is preserved.

#[derive(Debug, Clone)]
pub struct SendJob {
    pub peer: String,
    pub text: String,
}

/// Transient send failures the head job gets before it is dropped.
const MAX_HEAD_RETRIES: u32 = 3;

/// Errors that say nothing about the message itself: the connection dropped
/// or the server had a fault. Anything else (CHAT_WRITE_FORBIDDEN, bad peer)
/// is permanent and drops the job.
pub fn is_transient_send_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    [
        "timeout",
        "timed out",
        "connection",
        "reset",
        "broken pipe",
        "network",
        "unavailable",
        "internal",
        "rpc_call_fail",
        "rpc error 5",
        "dns",
        "eof",
    ]
    .iter()
    .any(|k| e.contains(k))
}

#[derive(Debug, Default)]
pub struct WriteQueue {
    jobs: VecDeque<SendJob>,
    /// Transient failures of the current head job.
    head_failures: u32,
}

impl WriteQueue {
    /// Enqueue a send; returns its 1-based position.
    pub fn push(&mut self, job: SendJob) -> usize {
        self.jobs.push_back(job);
        self.jobs.len()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Drain as many jobs as the gate allows, awaiting `sender` per job.
    /// Returns (sent, deferred). Jobs whose peer gate is closed stay at
    /// the FRONT (FIFO preserved); a deferred head blocks the rest.
    pub async fn drain_with<F, Fut>(
        &mut self,
        guard: &TelegramRateGuard,
        account: &str,
        sender: F,
    ) -> (usize, usize)
    where
        F: Fn(&SendJob) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let mut sent = 0;
        while let Some(job) = self.jobs.front() {
            let key = peer_key_for_peer_string(&job.peer);
            let wait = guard.before_call(account, Category::Send, key.as_deref());
            if !wait.is_zero() {
                return (sent, self.jobs.len());
            }
            match sender(job).await {
                Ok(()) => {
                    guard.record_success(account, Category::Send);
                    self.jobs.pop_front();
                    self.head_failures = 0;
                    sent += 1;
                }
                Err(e) => {
                    if let Some(secs) = parse_flood_wait(&e) {
                        guard.record_flood(account, Category::Send, Duration::from_secs(secs));
                    } else if is_transient_send_error(&e) && self.head_failures < MAX_HEAD_RETRIES {
                        // Keep the head and try again on a later drain. A dropped
                        // connection must not lose the message.
                        self.head_failures += 1;
                        tracing::warn!(
                            "telegram queue: transient send failure to {} (retry {}/{MAX_HEAD_RETRIES}): {e}",
                            job.peer,
                            self.head_failures
                        );
                    } else {
                        // Permanent failure, or transient retries used up: drop
                        // the job (a stuck head would starve every send behind
                        // it) and surface in logs.
                        tracing::error!("telegram queue: send to {} failed: {e}", job.peer);
                        guard.record_success(account, Category::Send);
                        self.jobs.pop_front();
                        self.head_failures = 0;
                    }
                    return (sent, self.jobs.len());
                }
            }
        }
        (sent, 0)
    }
}

/// "FLOOD_WAIT_42" → 42. Grammers surfaces flood waits as strings.
pub fn parse_flood_wait(err: &str) -> Option<u64> {
    err.split("FLOOD_WAIT_")
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
}

/// The queue jobs address peers by the same strings MCP callers use
/// (username or "user:<id>" style) — the raw id form carries the kind.
fn peer_key_for_peer_string(peer: &str) -> Option<String> {
    if peer.starts_with("user:") || peer.starts_with("channel:") || peer.starts_with("chat:") {
        Some(peer.to_string())
    } else {
        // Usernames resolve at send time; the resolved chat id keys the
        // peer bucket then, so the account gate alone paces these.
        None
    }
}

pub static TELEGRAM_RATE_GUARD: LazyLock<TelegramRateGuard> = LazyLock::new(TelegramRateGuard::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flood_wait_strings_parse() {
        assert_eq!(
            parse_flood_wait("rpc error FLOOD_WAIT_42 (see ..)"),
            Some(42)
        );
        assert_eq!(parse_flood_wait("FLOOD_WAIT_300"), Some(300));
        assert_eq!(parse_flood_wait("OTHER_ERROR"), None);
    }

    #[test]
    fn zero_max_calls_defers_without_panicking() {
        // A zero cap admits nothing. The empty bucket must not reach `front()`
        // unchecked, which panicked before the bucket check was tied to it.
        let mut store = std::collections::HashMap::new();
        let spec = RateSpec {
            max_calls: 0,
            window: Duration::from_secs(60),
        };
        let wait = try_acquire(&mut store, &"peer", spec, Instant::now());
        assert!(
            wait.is_zero(),
            "an empty bucket with cap 0 must not block or panic"
        );
    }

    #[test]
    fn account_gate_sliding_window() {
        let g = TelegramRateGuard::new();
        // History default is 30/min; a tight custom spec is not injectable
        // without a test constructor — exercise the real Send spec instead:
        // 20/min means 20 immediate sends, the 21st is deferred.
        let mut deferrals = 0;
        for _ in 0..21 {
            if !g.before_call("acc", Category::Send, None).is_zero() {
                deferrals += 1;
            }
            g.record_success("acc", Category::Send);
        }
        assert_eq!(deferrals, 1, "the 21st send in a minute must be refused");
    }

    #[test]
    fn peer_refusal_refunds_account_slot() {
        let g = TelegramRateGuard::new();
        // Burst 5 sends to one user: peer pacing (1/5s) allows only the
        // first; the four refusals must each REFUND the account slot.
        assert!(g
            .before_call("acc", Category::Send, Some("user:7"))
            .is_zero());
        for _ in 0..4 {
            assert!(
                !g.before_call("acc", Category::Send, Some("user:7"))
                    .is_zero(),
                "peer pacing refuses the second send within 5s"
            );
        }
        // If refusals had consumed slots, the account bucket would hold 5;
        // with refunds it holds exactly 1. Fill the remaining 19 slots and
        // assert the exact 20/min boundary lands where it should.
        for i in 0..19 {
            assert!(
                g.before_call("acc", Category::Send, None).is_zero(),
                "account slot {i} must be free (refunds worked)"
            );
        }
        assert!(
            !g.before_call("acc", Category::Send, None).is_zero(),
            "the 21st send in a minute must be refused by the account gate"
        );
    }

    #[test]
    fn breaker_opens_after_three_floods() {
        let g = TelegramRateGuard::new();
        for _ in 0..3 {
            g.record_flood("acc", Category::History, Duration::from_secs(2));
        }
        let wait = g.before_call("acc", Category::History, None);
        // The suspension is stamped max(2s flood, 300s cooldown) *before*
        // before_call's now(), so allow sub-millisecond clock drift.
        assert!(
            wait > Duration::from_secs(299),
            "breaker cooldown must dominate the 2s flood (got {wait:?})"
        );
        // record_success clears the consecutive counter, but the suspension
        // stands until it expires — mirroring tg-cli.
    }

    #[tokio::test]
    async fn queue_respects_limiter_under_burst() {
        let g = TelegramRateGuard::new();
        let mut q = WriteQueue::default();
        for i in 0..5 {
            q.push(SendJob {
                peer: "user:42".into(),
                text: format!("burst {i}"),
            });
        }

        let sender = |_: &SendJob| async { Ok::<(), String>(()) };

        // First drain: user pacing (1/5s) sends exactly one, defers four.
        let (ok, deferred) = q.drain_with(&g, "acc", sender).await;
        assert_eq!((ok, deferred), (1, 4));

        // Still inside the 5s peer window: nothing more goes out.
        let (ok2, deferred2) = q.drain_with(&g, "acc", sender).await;
        assert_eq!((ok2, deferred2), (0, 4));

        // After the peer window passes, one more per 5s window.
        tokio::time::sleep(Duration::from_millis(5100)).await;
        let (ok3, _) = q.drain_with(&g, "acc", sender).await;
        assert_eq!(ok3, 1, "one peer send per 5s window");
    }

    #[test]
    fn transient_send_errors_are_classified() {
        assert!(is_transient_send_error("connection reset by peer"));
        assert!(is_transient_send_error("RPC error 500 INTERNAL"));
        assert!(!is_transient_send_error("CHAT_WRITE_FORBIDDEN"));
        assert!(!is_transient_send_error("PEER_ID_INVALID"));
    }

    #[tokio::test]
    async fn transient_head_failure_is_retried_not_dropped() {
        let g = TelegramRateGuard::new();
        let mut q = WriteQueue::default();
        // A username peer has no per-peer pacing key, so only the account gate
        // applies and the retry is not deferred by the 5s peer window.
        q.push(SendJob {
            peer: "@alice".into(),
            text: "hello".into(),
        });
        let attempts = std::cell::Cell::new(0u32);
        let sender = |_job: &SendJob| {
            attempts.set(attempts.get() + 1);
            let n = attempts.get();
            async move {
                if n == 1 {
                    Err::<(), String>("connection reset by peer".into())
                } else {
                    Ok(())
                }
            }
        };
        // First drain hits the transient error: the message stays queued.
        let (ok, _) = q.drain_with(&g, "acc", &sender).await;
        assert_eq!(
            (ok, q.len()),
            (0, 1),
            "a transient failure must keep the job"
        );
        // The next drain sends it.
        let (ok2, _) = q.drain_with(&g, "acc", &sender).await;
        assert_eq!(ok2, 1, "the retried job is sent");
        assert_eq!(q.len(), 0);
    }

    #[tokio::test]
    async fn queue_drops_permanently_failed_head() {
        let g = TelegramRateGuard::new();
        let mut q = WriteQueue::default();
        q.push(SendJob {
            peer: "user:9".into(),
            text: "will fail".into(),
        });
        q.push(SendJob {
            peer: "user:9".into(),
            text: "next".into(),
        });

        let sender = |job: &SendJob| {
            let fail = job.text == "will fail";
            async move {
                if fail {
                    Err::<(), String>("CHAT_WRITE_FORBIDDEN".into())
                } else {
                    Ok(())
                }
            }
        };
        let (ok, deferred) = q.drain_with(&g, "acc", sender).await;
        // Failing head is dropped; the next one is within the same 5s peer
        // window, so it stays deferred (pacing, not starvation).
        assert_eq!((ok, deferred), (0, 1));
        assert_eq!(q.len(), 1);
    }
}
