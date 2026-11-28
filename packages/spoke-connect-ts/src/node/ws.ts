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
