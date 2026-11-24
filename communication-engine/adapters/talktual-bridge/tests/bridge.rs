//! Bridge tests over a real WebSocket on 127.0.0.1.
//!
//! * Protocol tests use CE's **scripted** adapter (no model).
//! * `#[ignore]`d tests use the **real model** (`CE_SHERPA_MODEL_DIR`) on the
//!   synthetic TTS fixtures; run with `cargo test -- --ignored`.

use std::sync::Arc;
use std::time::Duration;

use ce_core::speech::{LanguageSupport, LanguageTag};
use ce_speech::AsrFactory;
use ce_speech::fake::{ScriptedFactory, UtteranceScript};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use talktual_bridge::{Bridge, BridgeConfig};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(factory: impl AsrFactory + 'static) -> (String, Arc<Bridge>) {
    let bridge = Bridge::start(factory, BridgeConfig::default())
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(bridge.clone().serve(listener));
    (url, bridge)
}

async fn connect(
    url: &str,
    origin: Option<&str>,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = url.into_client_request().unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn send_json(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// Send `samples` as 20 ms little-endian i16 frames (as fast as possible).
async fn send_pcm(ws: &mut Ws, samples: &[f32], rate: u32) {
    send_pcm_paced(ws, samples, rate, false).await;
}

/// Send 20 ms frames, optionally in real time like Talktual's AudioWorklet.
async fn send_pcm_paced(ws: &mut Ws, samples: &[f32], rate: u32, realtime: bool) {
    let frame = (rate / 50) as usize;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for chunk in samples.chunks(frame) {
        if realtime {
            tick.tick().await;
        }
        let bytes: Vec<u8> = chunk
            .iter()
            .flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        ws.send(Message::Binary(bytes.into())).await.unwrap();
    }
}

/// Read messages until `end` (or a timeout); returns them all.
async fn collect(ws: &mut Ws) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(30), ws.next())
            .await
            .expect("bridge answers in time");
        let Some(Ok(Message::Text(t))) = m else {
            break;
        };
        let v: Value = serde_json::from_str(&t).unwrap();
        let end = v["type"] == "end";
        out.push(v);
        if end {
            break;
        }
    }
    out
}

async fn ready(ws: &mut Ws) -> Value {
    let m = tokio::time::timeout(Duration::from_secs(30), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(m.to_text().unwrap()).unwrap()
}

fn scripted() -> ScriptedFactory {
    ScriptedFactory::new(vec![UtteranceScript::progressive("hello world again", 200)])
        .with_capabilities(|c| {
            c.languages = LanguageSupport::Fixed(vec![LanguageTag::parse("en").unwrap()]);
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_streams_through_ce_and_returns_partials_and_a_final() {
    let (url, _b) = serve(scripted()).await;
    let mut ws = connect(&url, Some("http://localhost:5173")).await.unwrap();
    send_json(
        &mut ws,
        serde_json::json!({"type":"start","lang":"en-US","sampleRate":48000}),
    )
    .await;
    let r = ready(&mut ws).await;
    assert_eq!(r["type"], "ready");
    assert_eq!(r["adapter"], "ce.asr.scripted");
    assert_eq!(r["segmentation"], "continuous");
    // 1.2 s of audio at 48 kHz, then a graceful stop.
    send_pcm(&mut ws, &vec![0.01; 57_600], 48_000).await;
    send_json(&mut ws, serde_json::json!({"type":"stop"})).await;
    let msgs = collect(&mut ws).await;
    let kinds: Vec<&str> = msgs.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"utterance"), "{kinds:?}");
    assert!(kinds.contains(&"partial"), "{kinds:?}");
    let fin = msgs.iter().find(|m| m["type"] == "final").expect("a final");
    assert_eq!(fin["text"], "hello world again");
    assert_eq!(fin["reason"], "streamend");
    assert!(fin["durationMs"].as_u64().unwrap() >= 1_000, "{fin}");
    assert_eq!(kinds.last(), Some(&"end"));
    assert_eq!(msgs.last().unwrap()["aborted"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn origins_languages_rates_and_abort_are_enforced() {
    let (url, _b) = serve(scripted()).await;
    // A foreign web page cannot connect.
    assert!(connect(&url, Some("https://evil.example")).await.is_err());
    // Unsupported language: fatal error, nothing is opened.
    let mut ws = connect(&url, Some("http://127.0.0.1:5173")).await.unwrap();
    send_json(
        &mut ws,
        serde_json::json!({"type":"start","lang":"fr-FR","sampleRate":48000}),
    )
    .await;
    let e = ready(&mut ws).await;
    assert_eq!(
        (e["type"].as_str(), e["code"].as_str(), e["fatal"].as_bool()),
        (Some("error"), Some("language-unsupported"), Some(true))
    );
    // Bad rate.
    let mut ws = connect(&url, None).await.unwrap();
    send_json(
        &mut ws,
        serde_json::json!({"type":"start","lang":"en","sampleRate":1000}),
    )
    .await;
    assert_eq!(ready(&mut ws).await["code"], "bridge.bad_rate");
    // Abort discards: no final, an aborted end.
    let mut ws = connect(&url, None).await.unwrap();
    send_json(
        &mut ws,
        serde_json::json!({"type":"start","lang":"en","sampleRate":16000}),
    )
    .await;
    assert_eq!(ready(&mut ws).await["type"], "ready");
    send_pcm(&mut ws, &vec![0.0; 8_000], 16_000).await;
    send_json(&mut ws, serde_json::json!({"type":"abort"})).await;
    let msgs = collect(&mut ws).await;
    assert_eq!(msgs.last().unwrap()["aborted"], true);
    // Malformed messages are reported, not fatal.
    let mut ws = connect(&url, None).await.unwrap();
    send_json(&mut ws, serde_json::json!({"type":"nope"})).await;
    let e = ready(&mut ws).await;
    assert_eq!(
        (e["code"].as_str(), e["fatal"].as_bool()),
        (Some("bridge.bad_message"), Some(false))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_pushes_are_independent_sessions() {
    let (url, _b) = serve(scripted()).await;
    let mut tasks = Vec::new();
    for _ in 0..3 {
        let url = url.clone();
        tasks.push(tokio::spawn(async move {
            let mut ws = connect(&url, None).await.unwrap();
            send_json(
                &mut ws,
                serde_json::json!({"type":"start","lang":"en","sampleRate":16000}),
            )
            .await;
            assert_eq!(ready(&mut ws).await["type"], "ready");
            send_pcm(&mut ws, &vec![0.01; 16_000], 16_000).await;
            send_json(&mut ws, serde_json::json!({"type":"stop"})).await;
            let msgs = collect(&mut ws).await;
            msgs.iter().filter(|m| m["type"] == "final").count()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), 1);
    }
}

fn real() -> ce_asr_sherpa::SherpaFactory {
    let dir =
        ce_asr_sherpa::model_dir_from_env().expect("CE_SHERPA_MODEL_DIR must point at the model");
    ce_asr_sherpa::SherpaFactory::new(ce_asr_sherpa::SherpaConfig::zipformer_en_2023_06_26_int8(
        dir,
    ))
}

fn fixture(id: &str) -> Vec<f32> {
    let (_, s) = ce_speech::eval::load_split(ce_speech::eval::Split::Dev)
        .unwrap()
        .into_iter()
        .chain(ce_speech::eval::load_split(ce_speech::eval::Split::Test).unwrap())
        .find(|(m, _)| m.id == id)
        .unwrap();
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the sherpa-onnx model (CE_SHERPA_MODEL_DIR)"]
async fn the_real_model_recognizes_a_fixture_streamed_like_talktual_does() {
    let (url, _b) = serve(real()).await;
    for (seg, rate) in [("continuous", 16_000u32), ("activity", 48_000)] {
        let mut ws = connect(&url, Some("http://localhost:5173")).await.unwrap();
        send_json(
            &mut ws,
            serde_json::json!({"type":"start","lang":"en-US","sampleRate":rate,"segmentation":seg}),
        )
        .await;
        let r = ready(&mut ws).await;
        assert_eq!(r["adapter"], "ce.asr.sherpa-zipformer", "{r}");
        let mut audio = vec![0.0f32; 8_000];
        audio.extend(fixture("dev_kal16_station"));
        audio.extend(vec![0.0f32; 16_000]);
        let audio = ce_audio_intel::synth::resample_linear(&audio, 16_000, rate);
        send_pcm_paced(&mut ws, &audio, rate, true).await;
        send_json(&mut ws, serde_json::json!({"type":"stop"})).await;
        let msgs = collect(&mut ws).await;
        let text: Vec<&str> = msgs
            .iter()
            .filter(|m| m["type"] == "final")
            .filter_map(|m| m["text"].as_str())
            .collect();
        let joined = text.join(" ");
        assert!(
            joined.contains("TRAIN STATION"),
            "{seg} @ {rate}: {joined:?}"
        );
        assert!(
            msgs.iter().any(|m| m["type"] == "partial"),
            "{seg}: partials"
        );
        if seg == "activity" {
            assert!(
                msgs.iter().any(|m| m["type"] == "activity"),
                "Phase 3 activity reported"
            );
        }
    }
}
