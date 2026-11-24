use std::sync::Arc;
use std::time::Duration;

use ce_core::event::EventEnvelope;
use communication_engine::{EventSubscription, SubscriptionError};

/// Collects events from a subscription and waits for specific ones.
#[derive(Debug)]
pub struct EventRecorder {
    subscription: EventSubscription,
    seen: Vec<Arc<EventEnvelope>>,
    lagged: u64,
    closed: bool,
}

impl EventRecorder {
    /// Wrap a subscription.
    pub fn new(subscription: EventSubscription) -> Self {
        Self {
            subscription,
            seen: Vec::new(),
            lagged: 0,
            closed: false,
        }
    }

    /// Wait up to `timeout` for an event matching `pred`. Every event
    /// received meanwhile is recorded.
    pub async fn wait_for(
        &mut self,
        timeout: Duration,
        mut pred: impl FnMut(&EventEnvelope) -> bool,
    ) -> Option<Arc<EventEnvelope>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let next = tokio::time::timeout_at(deadline, self.subscription.recv()).await;
            match next {
                Err(_) => return None,
                Ok(Ok(event)) => {
                    self.seen.push(event.clone());
                    if pred(&event) {
                        return Some(event);
                    }
                }
                Ok(Err(SubscriptionError::Lagged { missed })) => self.lagged += missed,
                Ok(Err(SubscriptionError::Closed)) => {
                    self.closed = true;
                    return None;
                }
            }
        }
    }

    /// Wait for an event of the given kind.
    pub async fn wait_for_kind(
        &mut self,
        timeout: Duration,
        kind: &str,
    ) -> Option<Arc<EventEnvelope>> {
        self.wait_for(timeout, |e| e.kind() == kind).await
    }

    /// Receive until the stream closes (runtime gone) or `timeout` elapses.
    /// Returns true if it closed.
    pub async fn drain_until_closed(&mut self, timeout: Duration) -> bool {
        self.wait_for(timeout, |_| false).await;
        self.closed
    }

    /// Everything recorded so far.
    pub fn events(&self) -> &[Arc<EventEnvelope>] {
        &self.seen
    }

    /// Kinds of everything recorded so far.
    pub fn kinds(&self) -> Vec<&'static str> {
        self.seen.iter().map(|e| e.kind()).collect()
    }

    /// Total events reported missed through `Lagged`.
    pub fn lagged(&self) -> u64 {
        self.lagged
    }

    /// Whether the stream has closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}
