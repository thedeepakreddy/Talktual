//! Wire protocol between Talktual (browser) and the bridge (version 1).
//!
//! One WebSocket per push / listening session:
//!
//! ```text
//! client → {"type":"start","lang":"en-US","sampleRate":48000,"segmentation":"continuous"}
//! bridge → {"type":"ready", ...}
//! client → binary frames: mono PCM, signed 16-bit little-endian, at sampleRate
//! bridge → {"type":"utterance"} {"type":"partial"} … {"type":"final"} {"type":"activity"}
//! client → {"type":"stop"}        (graceful: everything sent so far is recognized)
//! bridge → remaining finals, then {"type":"end"}
//! client → {"type":"abort"}       (discard; the bridge replies {"type":"end","aborted":true})
//! ```
//!
//! Optional diagnostics (the Engine Lab page): `{"type":"info"}` and
//! `{"type":"health"}` at any time; `"diagnostics": true` in `start` adds every
//! CE event of the session (`ce-event`) and a status snapshot every second
//! (`status`); `"analysis": true` runs Phase 3 analysis in continuous mode
//! too; `{"type":"reset"}` resets the recognizer. Clients that do not ask
//! for diagnostics see exactly the v1 messages above.
//!
//! Text is the recognizer's own output (the sherpa-onnx model emits
//! upper-case words without punctuation); presentation is the client's job.

use serde::{Deserialize, Serialize};

/// Protocol version.
pub const PROTOCOL: u32 = 1;

/// How utterances are delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Segmentation {
    /// Feed everything; utterances end on the model's endpoint or the end
    /// of input (push-to-talk).
    #[default]
    Continuous,
    /// Phase 3 activity hints delimit utterances (hands-free listening).
    Activity,
}

/// Client → bridge text messages.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMsg {
    /// Begin a session.
    Start {
        /// BCP-47 tag requested by the user.
        lang: String,
        /// Rate of the PCM frames that follow.
        #[serde(rename = "sampleRate")]
        sample_rate: u32,
        /// Delimitation.
        #[serde(default)]
        segmentation: Segmentation,
        /// Stream every CE event of the session and periodic status.
        #[serde(default)]
        diagnostics: bool,
        /// Run Phase 3 analysis even with continuous delimitation.
        #[serde(default)]
        analysis: bool,
    },
    /// Graceful end.
    Stop,
    /// Discard and end.
    Abort,
    /// Recognizers, capabilities and runtime health (no audio needed).
    Info,
    /// Runtime health now.
    Health,
    /// Cancel the open utterance and reset the recognizer.
    Reset,
}

/// Bridge → client messages.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMsg {
    /// Recognition is running; send audio.
    Ready {
        /// Protocol version.
        protocol: u32,
        /// Adapter id.
        adapter: String,
        /// Model id, if any.
        model: Option<String>,
        /// Delimitation in force.
        segmentation: Segmentation,
    },
    /// An utterance opened.
    Utterance {
        /// Utterance number.
        utterance: u64,
    },
    /// A partial revision (full snapshot).
    Partial {
        /// Utterance.
        utterance: u64,
        /// Revision number.
        revision: u32,
        /// Stable text.
        committed: String,
        /// Volatile tail.
        provisional: String,
    },
    /// The final text of an utterance.
    Final {
        /// Utterance.
        utterance: u64,
        /// Text.
        text: String,
        /// Why it ended.
        reason: String,
        /// Utterance language decision (a tag) or null when CE abstained.
        language: Option<String>,
        /// Audio covered, ms.
        #[serde(rename = "durationMs")]
        duration_ms: u64,
    },
    /// An utterance was abandoned.
    Cancelled {
        /// Utterance.
        utterance: u64,
        /// Why.
        reason: String,
    },
    /// Phase 3 activity changed (activity segmentation only).
    Activity {
        /// Probable speech now.
        active: bool,
    },
    /// Non-fatal condition (dropped audio, lagging events).
    Warning {
        /// Kind.
        kind: String,
    },
    /// An error.
    Error {
        /// Stable code.
        code: String,
        /// Operational message.
        message: String,
        /// The session cannot continue.
        fatal: bool,
    },
    /// The session ended; nothing follows.
    End {
        /// Ended by `abort` (results discarded).
        aborted: bool,
    },
    /// Engine information (reply to `info`).
    Info {
        /// Protocol version.
        protocol: u32,
        /// Registered recognizers (`AsrCapabilities`, CE JSON).
        recognizers: serde_json::Value,
        /// Runtime health (`RuntimeHealth`, CE JSON).
        health: serde_json::Value,
    },
    /// Runtime health (reply to `health`).
    Health {
        /// `RuntimeHealth`, CE JSON.
        health: serde_json::Value,
    },
    /// One CE event of this session (diagnostics).
    #[serde(rename = "ce-event")]
    CeEvent {
        /// Runtime sequence number.
        sequence: u64,
        /// Payload type (`audio`, `analysis`, `speech`, `session_created`, …).
        topic: String,
        /// Inner event kind (`transcript`, `activity_changed`, …) or the topic.
        kind: String,
        /// Privacy class of the payload.
        privacy: String,
        /// The event, CE JSON.
        data: serde_json::Value,
    },
    /// Periodic snapshot (diagnostics).
    Status {
        /// `AudioStreamStatus`.
        audio: serde_json::Value,
        /// `AnalysisStatus`, if analysis runs.
        analysis: serde_json::Value,
        /// `RecognitionStatus`.
        recognition: serde_json::Value,
        /// Engine-wide `MetricsSnapshot`.
        metrics: serde_json::Value,
    },
}

/// Decode little-endian i16 PCM (an odd trailing byte is ignored).
pub fn decode_pcm(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_in_the_documented_shape() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"start","lang":"en-US","sampleRate":48000,"segmentation":"activity"}"#,
        )
        .unwrap();
        assert_eq!(
            m,
            ClientMsg::Start {
                lang: "en-US".into(),
                sample_rate: 48_000,
                segmentation: Segmentation::Activity,
                diagnostics: false,
                analysis: false,
            }
        );
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"start","lang":"en","sampleRate":16000}"#).unwrap();
        assert!(matches!(
            m,
            ClientMsg::Start {
                segmentation: Segmentation::Continuous,
                ..
            }
        ));
        assert_eq!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"stop"}"#).unwrap(),
            ClientMsg::Stop
        );
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"bogus"}"#).is_err());
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"start","lang":"en","sampleRate":16000,"diagnostics":true,"analysis":true}"#,
        )
        .unwrap();
        assert!(matches!(
            m,
            ClientMsg::Start {
                diagnostics: true,
                analysis: true,
                ..
            }
        ));
        assert_eq!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"info"}"#).unwrap(),
            ClientMsg::Info
        );
        let e = serde_json::to_value(ServerMsg::CeEvent {
            sequence: 3,
            topic: "speech".into(),
            kind: "transcript".into(),
            privacy: "content".into(),
            data: serde_json::json!({}),
        })
        .unwrap();
        assert_eq!(e["type"], "ce-event");
        let s = serde_json::to_value(ServerMsg::Final {
            utterance: 1,
            text: "HELLO".into(),
            reason: "stream_end".into(),
            language: None,
            duration_ms: 900,
        })
        .unwrap();
        assert_eq!(s["type"], "final");
        assert_eq!(s["durationMs"], 900);
        assert_eq!(decode_pcm(&[1, 0, 0xff, 0x7f, 9]), vec![1, i16::MAX]);
    }
}
