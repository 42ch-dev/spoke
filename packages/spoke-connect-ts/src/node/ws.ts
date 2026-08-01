/**
 * Node-only WebSocket adapter for SPOKE connect framing (AD-P0-5: `ws`
 * behind a `node` subpath). The `ws` import is isolated here so the
 * isomorphic `src/framing.ts` stays dependency-free for browser builds
 * (native WebSocket swap-in later). Not part of the public barrel
 * (AD-P0-6) — consumers import this subpath directly.
 *
 * Wire contract: one JSON document per WebSocket message (text frames),
 * ordered reliable stream, no batching (AD-P0-3).
 */

import { WebSocket } from "ws";
import type { RawData } from "ws";

import { decodeJsonMessage, encodeJsonMessage } from "../framing.js";

/** Send one JSON document as a single text frame. */
export function sendJsonMessage(socket: WebSocket, doc: unknown): void {
  socket.send(encodeJsonMessage(doc));
}

/**
 * Attach a one-JSON-document-per-message receiver. Text frames arrive as
 * `Buffer` (UTF-8) and binary frames as `Buffer` / `ArrayBuffer`; both
 * decode as exactly one JSON document via the isomorphic codec.
 *
 * A decode failure (malformed JSON frame) never escapes into the `ws`
 * message listener — a throw there would surface as an uncaught exception
 * and crash the host process. The error is routed to `onDecodeError` when
 * provided (the client transport boundary maps it to fail-all + close,
 * mirroring the handshake-rejection pattern); without a handler the frame
 * is dropped.
 */
export function onJsonMessage(
  socket: WebSocket,
  handler: (doc: unknown) => void,
  onDecodeError?: (error: Error) => void,
): void {
  socket.on("message", (data: RawData) => {
    const payload = Array.isArray(data) ? Buffer.concat(data) : data;
    let doc: unknown;
    try {
      doc = decodeJsonMessage(payload);
    } catch (error) {
      onDecodeError?.(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    handler(doc);
  });
}
