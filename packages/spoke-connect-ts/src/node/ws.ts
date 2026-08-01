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
 */
export function onJsonMessage(
  socket: WebSocket,
  handler: (doc: unknown) => void,
): void {
  socket.on("message", (data: RawData) => {
    const payload = Array.isArray(data) ? Buffer.concat(data) : data;
    handler(decodeJsonMessage(payload));
  });
}
