//! The capture authorization gate: denial by default, every denial reason,
//! revocation while streaming, platform permission changes, stale consent
//! and authorization/start races.

mod common;

use std::time::Duration;

use ce_audio::StreamFault;
use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::audio::{
    AudioEvent, AuthorizationDenial, PlatformPermission, TerminalReason,
};
use communication_engine::prelude::*;

fn denial_of(err: &CeError) -> String {
    err.context()
        .iter()
        .find(|f| f.key == "denial")
        .map(|f| f.value.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn capture_is_denied_by_default_and_nothing_is_opened() {
    let mut h = audio_harness().await;
    let err = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Unauthorized, "audio.capture_unauthorized")
    );
    assert_eq!(denial_of(&err), "consent_absent");
    assert!(h.fake.streams().is_empty(), "no device was opened");
    assert_eq!(h.engine.metrics().audio_capture_denied, 1);
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::CaptureDenied { .. }))
        .await;
    assert_eq!(
        e,
        AudioEvent::CaptureDenied {
            stream: None,
            denial: AuthorizationDenial::ConsentAbsent
        }
    );
    // Playback needs no capture consent.
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_consent_and_permission_state_other_than_granted_denies() {
    let h = audio_harness().await;
    let at = h.now();
    let consent_cases = [
        (ConsentState::Denied, "consent_denied"),
        (ConsentState::Revoked { at }, "consent_revoked"),
        (ConsentState::Unknown, "consent_absent"),
    ];
    for (state, expected) in consent_cases {
        h.engine
            .update_consent(h.session.id, ConsentPurpose::Capture, state)
            .await
            .unwrap();
        let err = h
            .engine
            .open_capture(h.session.id, StreamRequest::capture())
            .await
            .unwrap_err();
        assert_eq!(denial_of(&err), expected);
    }
    h.grant_capture().await;
    let permission_cases = [
        (PlatformPermission::Denied, "permission_denied"),
        (
            PlatformPermission::NotDetermined,
            "permission_not_determined",
        ),
        (PlatformPermission::Restricted, "permission_restricted"),
        (PlatformPermission::Unknown, "permission_unknown"),
    ];
    for (p, expected) in permission_cases {
        h.fake.set_permission_silently(p);
        let err = h
            .engine
            .open_capture(h.session.id, StreamRequest::capture())
            .await
            .unwrap_err();
        assert_eq!(denial_of(&err), expected, "{p:?}");
    }
    // A platform without a permission model allows capture (with consent).
    h.fake
        .set_permission_silently(PlatformPermission::NotRequiredByPlatform);
    h.engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    assert_eq!(
        h.fake.streams().len(),
        1,
        "only the authorized attempt opened a device"
    );
    // Consent for other purposes does not authorize capture.
    let s2 = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.engine
        .update_consent(
            s2.id,
            ConsentPurpose::RemoteProcessing,
            ConsentState::Granted { at },
        )
        .await
        .unwrap();
    assert_eq!(
        denial_of(
            &h.engine
                .open_capture(s2.id, StreamRequest::capture())
                .await
                .unwrap_err()
        ),
        "consent_absent"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn revoking_consent_stops_capture_immediately_and_releases_the_device() {
    let mut h = audio_harness().await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    for _ in 0..3 {
        h.fake.tick(480);
    }
    // Wait until blocks are queued but do not read them.
    tokio::time::timeout(WAIT, async {
        while reader.queued() < 3 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let fake_stream = h.fake.last_stream().unwrap();
    h.engine
        .update_consent(
            h.session.id,
            ConsentPurpose::Capture,
            ConsentState::Revoked { at: h.now() },
        )
        .await
        .unwrap();
    // Queued audio is discarded, not delivered.
    assert_eq!(reader.recv().await.unwrap_err(), ReadError::Revoked);
    assert!(
        fake_stream.is_closed() && !fake_stream.holds_endpoint(),
        "device released"
    );
    assert!(h.engine.audio_streams().await.unwrap().is_empty());
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::CaptureRevoked { .. }))
        .await;
    assert_eq!(
        e,
        AudioEvent::CaptureRevoked {
            stream: id,
            denial: AuthorizationDenial::ConsentRevoked
        }
    );
    assert_eq!(h.engine.metrics().audio_capture_revoked, 1);
    // Re-granting does not resurrect the stream; a new one must be opened.
    h.grant_capture().await;
    assert_eq!(
        h.engine.start_audio(id).await.unwrap_err().code(),
        "audio.stream_not_found"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn platform_permission_revocation_is_enforced_whether_notified_or_polled() {
    for notified in [true, false] {
        let mut h = audio_harness().await;
        h.grant_capture().await;
        let mut reader = h
            .engine
            .open_capture(h.session.id, StreamRequest::capture())
            .await
            .unwrap();
        h.engine.start_audio(reader.info().id).await.unwrap();
        // Playback is unaffected by capture permission.
        let playback = h
            .engine
            .open_playback(h.session.id, StreamRequest::playback())
            .await
            .unwrap();
        if notified {
            h.fake.set_permission(PlatformPermission::Denied);
        } else {
            // No notification: housekeeping (every 50 ms here) notices.
            h.fake.set_permission_silently(PlatformPermission::Denied);
        }
        h.wait_audio(|e| {
            matches!(
                e,
                AudioEvent::CaptureRevoked {
                    denial: AuthorizationDenial::PermissionDenied,
                    ..
                }
            )
        })
        .await;
        assert_eq!(
            tokio::time::timeout(WAIT, reader.recv())
                .await
                .unwrap()
                .unwrap_err(),
            ReadError::Revoked
        );
        let streams = h.engine.audio_streams().await.unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].info.id, playback.info().id);
        assert!(h.audio_events().iter().any(|e| matches!(
            e,
            AudioEvent::PermissionChanged {
                to: PlatformPermission::Denied,
                ..
            }
        )));
        h.engine.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn a_backend_permission_fault_revokes_capture() {
    let mut h = audio_harness().await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    h.fake
        .last_stream()
        .unwrap()
        .fault(StreamFault::PermissionRevoked);
    h.wait_audio(|e| matches!(e, AudioEvent::CaptureRevoked { .. }))
        .await;
    assert_eq!(
        tokio::time::timeout(WAIT, reader.recv())
            .await
            .unwrap()
            .unwrap_err(),
        ReadError::Revoked
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_consent_blocks_start_and_stops_a_running_stream() {
    let mut config = audio_config();
    config.audio.consent_max_age_ms = Some(1_000);
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.clock.advance(Duration::from_secs(2));
    let err = h.engine.start_audio(id).await.unwrap_err();
    assert_eq!(denial_of(&err), "consent_stale");
    h.wait_audio(|e| {
        matches!(
            e,
            AudioEvent::CaptureRevoked {
                denial: AuthorizationDenial::ConsentStale,
                ..
            }
        )
    })
    .await;
    assert_eq!(
        tokio::time::timeout(WAIT, reader.recv())
            .await
            .unwrap()
            .unwrap_err(),
        ReadError::Revoked
    );

    // Fresh consent; then it ages out while running.
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    h.clock.advance(Duration::from_millis(1_500));
    assert_eq!(
        tokio::time::timeout(WAIT, reader.recv())
            .await
            .unwrap()
            .unwrap_err(),
        ReadError::Revoked
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revocation_racing_an_open_never_yields_an_authorized_stream() {
    let h = audio_harness().await;
    h.grant_capture().await;
    h.fake.delay_open(Duration::from_millis(200));
    let engine = std::sync::Arc::new(h.engine);
    let opener = {
        let e = engine.clone();
        let session = h.session.id;
        tokio::spawn(async move { e.open_capture(session, StreamRequest::capture()).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    // The platform permission flips while the device is opening.
    h.fake.set_permission(PlatformPermission::Denied);
    let result = opener.await.unwrap();
    match result {
        Err(e) => assert_eq!(e.category(), ErrorCategory::Unauthorized),
        Ok(mut reader) => {
            // If the open won the race, the stream must already be revoked.
            assert_eq!(
                tokio::time::timeout(WAIT, reader.recv())
                    .await
                    .unwrap()
                    .unwrap_err(),
                ReadError::Revoked
            );
        }
    }
    tokio::time::timeout(WAIT, async {
        while !engine.audio_streams().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(h.fake.streams().iter().all(|s| s.is_closed()));
    let engine = std::sync::Arc::into_inner(engine).unwrap();
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consent_revoked_while_start_is_queued_wins_before_any_data_flows() {
    let h = audio_harness().await;
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    let at = h.now();
    let engine = std::sync::Arc::new(h.engine);
    // Commands are serialized by the core: whichever lands second sees the
    // first's effect. Either way no authorized running stream remains.
    let (start, revoke) = tokio::join!(
        {
            let e = engine.clone();
            async move { e.start_audio(id).await }
        },
        {
            let e = engine.clone();
            let s = h.session.id;
            async move {
                e.update_consent(s, ConsentPurpose::Capture, ConsentState::Revoked { at })
                    .await
            }
        },
    );
    revoke.unwrap();
    if let Ok(status) = start {
        assert_eq!(status.state, AudioStreamState::Running);
    }
    tokio::time::timeout(WAIT, async {
        while !engine.audio_streams().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut reader = reader;
    h.fake.tick(480);
    assert_eq!(reader.recv().await.unwrap_err(), ReadError::Revoked);
    let engine = std::sync::Arc::into_inner(engine).unwrap();
    let report = engine.shutdown().await.unwrap();
    assert!(report.clean);
    let _ = TerminalReason::Closed;
}
