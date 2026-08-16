/**
 * Demo server CLI — boot the mock inference host on a WebSocket and print
 * its identity, allowlist, and listening URL.
 *
 *   node dist/main.js [--port 8787]
 */

import { DEMO_CLIENT_PEER_ID } from "./identities.js";
import { serveConnectDemo } from "./transport/ws-server.js";

const DEFAULT_PORT = 8787;

function parsePort(argv: string[]): number {
  const flagIndex = argv.indexOf("--port");
  if (flagIndex === -1) {
    return DEFAULT_PORT;
  }
  const raw = argv[flagIndex + 1];
  const port = Number(raw);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`invalid --port value: ${raw ?? "(missing)"}`);
  }
  return port;
}

const port = parsePort(process.argv.slice(2));
const { url, peerId, close } = await serveConnectDemo({ port });

console.log("SPOKE connect demo — mock inference host");
console.log(`  peer_id:   ${peerId}`);
console.log(`  allowlist: ${DEMO_CLIENT_PEER_ID}`);
console.log(`  listening: ${url}`);
console.log(
  "  tools:     discovers dialer tools from the authenticated manifest;",
);
console.log("             reverse-invokes tools.toy_world.roll_dice mid-orchestration");
console.log("  (Ctrl+C to stop)");

function shutdown(): void {
  close();
  process.exit(0);
}
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
