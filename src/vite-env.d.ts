/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** WebSocket URL of the signaling server; its http(s) form serves the API proxy. */
  readonly VITE_SIGNALING_URL?: string;
  /** Optional TURN server, needed for P2P behind symmetric NAT. */
  readonly VITE_TURN_URL?: string;
  readonly VITE_TURN_USERNAME?: string;
  readonly VITE_TURN_CREDENTIAL?: string;
  /** "1" enables the VoiceMirror (Phase 3) prototype in this build. */
  readonly VITE_VOICEMIRROR?: string;
  /** Phase 4: "0"/"off" disables the conversation layer (rollback). */
  readonly VITE_CONVERSATION_LAYER?: string;
  /** Phase 5: "0"/"off" disables communication intelligence (rollback). */
  readonly VITE_COMMUNICATION_INTELLIGENCE?: string;
  /** Phase 6: "0"/"off" disables processing modes (rollback to Phase 5). */
  readonly VITE_PROCESSING_MODES?: string;
  /** Phase 6: "1" enables the on-device speech-recognition probe (off: crashed the tab on test hardware). */
  readonly VITE_ONDEVICE_ASR?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
