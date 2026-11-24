//! Scoped, bounded, expiring, ephemeral context (one per stream).
//!
//! * **Scope**: `Off`, `Utterance` (dropped when the utterance ends) or
//!   `Session` (the stream's semantic session). Nothing is persisted, and a
//!   manager never sees another stream's items. The runtime creates one
//!   manager per semantic worker; there is no global context.
//! * **Admission**: only entity observations whose span is committed
//!   (`Stable` or `Final`) and whose certainty is at least
//!   `min_certainty` become context. Provisional text never does.
//! * **Bounds**: `max_items` and `max_bytes` evict the oldest items first;
//!   `ttl_ms` expires items on the stream's audio timeline;
//!   `max_utterance_distance` limits which utterances are candidates.
//! * **Correction**: when a source observation is updated or retracted, its
//!   item is invalidated (removed and reported).
//! * **Teardown**: `clear` on reset, stop, cancellation and shutdown.
//!
//! Context events carry ids and sizes only, never mentions.

use std::collections::VecDeque;

use ce_core::ids::AudioStreamId;
use ce_core::privacy::DataClass;
use ce_core::semantic::{
    ContextChange, ContextConfig, ContextItem, ContextItemId, ContextScope, ContextScopeMode,
    ContextSize, ContextStatus, ObservationId, ObservationPayload, ObservationStatus,
    SemanticEvent, SemanticObservation,
};
use ce_core::time::MonotonicTime;

use crate::adapter::ContextView;

/// Per-stream context.
#[derive(Debug)]
pub struct ContextManager {
    stream: AudioStreamId,
    config: ContextConfig,
    items: VecDeque<ContextItem>,
    bytes: u32,
    next: u64,
    utterances: VecDeque<u64>,
    evicted: u64,
    expired: u64,
}

impl ContextManager {
    /// An empty context.
    pub fn new(stream: AudioStreamId, config: ContextConfig) -> Self {
        Self {
            stream,
            config,
            items: VecDeque::new(),
            bytes: 0,
            next: 0,
            utterances: VecDeque::new(),
            evicted: 0,
            expired: 0,
        }
    }

    /// Current size.
    pub fn size(&self) -> ContextSize {
        ContextSize {
            items: self.items.len() as u32,
            bytes: self.bytes,
        }
    }

    /// Items evicted by limits so far.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Items expired so far.
    pub fn expired(&self) -> u64 {
        self.expired
    }

    /// Active items (for tests and status).
    pub fn items(&self) -> impl Iterator<Item = &ContextItem> {
        self.items.iter()
    }

    fn event(&self, change: ContextChange, item: Option<ContextItemId>) -> SemanticEvent {
        SemanticEvent::Context {
            stream: self.stream,
            change,
            item,
            size: self.size(),
        }
    }

    /// Record that an utterance has begun (for the utterance distance).
    pub fn note_utterance(&mut self, utterance: u64) {
        if self.utterances.back() != Some(&utterance) && !self.utterances.contains(&utterance) {
            self.utterances.push_back(utterance);
            while self.utterances.len() > self.config.max_utterance_distance as usize + 1 {
                self.utterances.pop_front();
            }
        }
    }

    /// Admit an observation if it qualifies. Returns the new item id.
    pub fn admit(
        &mut self,
        obs: &SemanticObservation,
        now: MonotonicTime,
        out: &mut Vec<SemanticEvent>,
    ) -> Option<ContextItemId> {
        if self.config.scope == ContextScopeMode::Off {
            return None;
        }
        let ObservationPayload::Entity(e) = &obs.payload else {
            return None;
        };
        if !matches!(
            obs.status,
            ObservationStatus::Stable | ObservationStatus::Final
        ) || obs.uncertainty.level < self.config.min_certainty
            || self.items.iter().any(|i| i.observation == obs.id)
        {
            return None;
        }
        let bytes = (e.mention.len() + e.category.len() + 64) as u32;
        if bytes > self.config.max_bytes {
            return None;
        }
        self.next += 1;
        let id = ContextItemId(format!("ctx.{}", self.next));
        let scope = match self.config.scope {
            ContextScopeMode::Utterance => ContextScope::Utterance {
                utterance: obs.source.utterance,
            },
            _ => ContextScope::Session,
        };
        let ttl = self.config.ttl_ms.saturating_mul(1_000_000);
        self.items.push_back(ContextItem {
            id: id.clone(),
            scope,
            category: e.category.clone(),
            mention: e.mention.clone(),
            normalized: e.normalized.clone(),
            observation: obs.id.clone(),
            source: obs.source.clone(),
            certainty: obs.uncertainty.level,
            status: ContextStatus::Active,
            data_class: DataClass::Content,
            created_at: now,
            expires_at: MonotonicTime::from_nanos(now.as_nanos().saturating_add(ttl)),
            bytes,
        });
        self.bytes += bytes;
        out.push(self.event(ContextChange::Added, Some(id.clone())));
        while self.items.len() > self.config.max_items as usize
            || self.bytes > self.config.max_bytes
        {
            if let Some(old) = self.items.pop_front() {
                self.bytes -= old.bytes;
                self.evicted += 1;
                out.push(self.event(ContextChange::Evicted, Some(old.id)));
            } else {
                break;
            }
        }
        Some(id)
    }

    fn remove_where(
        &mut self,
        change: ContextChange,
        pred: impl Fn(&ContextItem) -> bool,
        out: &mut Vec<SemanticEvent>,
    ) -> Vec<ContextItemId> {
        let mut removed = Vec::new();
        let mut keep = VecDeque::with_capacity(self.items.len());
        for i in self.items.drain(..) {
            if pred(&i) {
                self.bytes -= i.bytes;
                removed.push(i.id);
            } else {
                keep.push_back(i);
            }
        }
        self.items = keep;
        for id in &removed {
            out.push(self.event(change, Some(id.clone())));
        }
        removed
    }

    /// Invalidate the item recorded from `observation` (it was corrected
    /// or retracted).
    pub fn invalidate_observation(
        &mut self,
        observation: &ObservationId,
        out: &mut Vec<SemanticEvent>,
    ) -> Vec<ContextItemId> {
        self.remove_where(
            ContextChange::Invalidated,
            |i| &i.observation == observation,
            out,
        )
    }

    /// Invalidate every item of an utterance (it was cancelled).
    pub fn invalidate_utterance(
        &mut self,
        utterance: u64,
        out: &mut Vec<SemanticEvent>,
    ) -> Vec<ContextItemId> {
        self.remove_where(
            ContextChange::Invalidated,
            |i| i.source.utterance == utterance,
            out,
        )
    }

    /// The utterance ended: utterance-scoped items are dropped.
    pub fn end_utterance(&mut self, utterance: u64, out: &mut Vec<SemanticEvent>) {
        if self.config.scope == ContextScopeMode::Utterance {
            self.remove_where(
                ContextChange::Cleared,
                |i| matches!(i.scope, ContextScope::Utterance { utterance: u } if u == utterance),
                out,
            );
        }
    }

    /// Expire items whose TTL has passed at `now` (stream audio time).
    pub fn expire(&mut self, now: MonotonicTime, out: &mut Vec<SemanticEvent>) {
        let n = self
            .remove_where(ContextChange::Expired, |i| i.expires_at <= now, out)
            .len();
        self.expired += n as u64;
    }

    /// Drop everything.
    pub fn clear(&mut self, out: &mut Vec<SemanticEvent>) {
        let had = !self.items.is_empty();
        self.items.clear();
        self.bytes = 0;
        self.utterances.clear();
        if had {
            out.push(self.event(ContextChange::Cleared, None));
        }
    }

    /// What an adapter may see while interpreting `utterance`: active items
    /// of permitted utterances, most recent first.
    pub fn view(&self, utterance: u64) -> ContextView {
        if self.config.scope == ContextScopeMode::Off {
            return ContextView {
                items: Vec::new(),
                disabled: true,
            };
        }
        let items = self
            .items
            .iter()
            .rev()
            .filter(|i| i.status == ContextStatus::Active)
            .filter(|i| {
                i.source.utterance == utterance || self.utterances.contains(&i.source.utterance)
            })
            .filter(|i| match i.scope {
                ContextScope::Utterance { utterance: u } => u == utterance,
                ContextScope::Session => true,
            })
            .cloned()
            .collect();
        ContextView {
            items,
            disabled: false,
        }
    }
}
