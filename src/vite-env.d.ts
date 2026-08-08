/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** WebSocket URL of the signaling server; its http(s) form serves the API proxy. */
  readonly VITE_SIGNALING_URL?: string;
  /** Optional TURN server, needed for P2P behind symmetric NAT. */
  readonly VITE_TURN_URL?: string;
  readonly VITE_TURN_USERNAME?: string;
  readonly VITE_TURN_CREDENTIAL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
