/**
 * SPOKE connect WebSocket framing — one JSON envelope per WebSocket message.
 *
 * Normative: `.mstar/specs/spoke-connect.md` §Transport framing (AD-P0-1 /
 * AD-P0-3): text frames, one JSON document per message, ordered reliable
 * stream, no batching.
 *
 * Isomorphic by design (AD-P0-5): this module touches only `string` /
 * `Uint8Array` / `ArrayBuffer` and never imports `ws`, so a browser build
 * can swap in the native WebSocket later. The Node `ws` adapter lives in
 * `src/node/ws.ts` (Node-only subpath) and consumes these functions.
 */

/**
 * Encode one JSON document as the payload of a single text frame.
 *
 * Fails when the document is not JSON-serializable (`JSON.stringify`
 * returns `undefined`) — a frame must carry exactly one JSON document.
 */
export function encodeJsonMessage(doc: unknown): string {
  const payload = JSON.stringify(doc);
  if (payload === undefined) {
    throw new Error("connect frame: document is not JSON-serializable");
  }
  return payload;
}

/**
 * Decode exactly one JSON document from a message payload.
 *
 * Accepts text frames (`string`) and binary frames (`Uint8Array` /
 * `ArrayBuffer`, decoded as UTF-8). `JSON.parse` consumes the entire
 * payload, so trailing bytes or a second document in one frame fail closed
 * (no batching, AD-P0-3).
 */
export function decodeJsonMessage(
  payload: string | Uint8Array | ArrayBuffer,
): unknown {
  const text =
    typeof payload === "string"
      ? payload
      : new TextDecoder().decode(payload as BufferSource);
  return JSON.parse(text);
}
