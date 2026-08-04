import { describe, expect, it } from "vitest";

// Package-name imports (self-reference) exercise the package `exports` map —
// both subpaths must resolve and expose their public surface. This
// machine-checks the published-shape contract before any Stage 1 publish.
import { derivePeerIdFromEd25519Pubkey } from "@42ch/spoke-connect";
import { connectClient } from "@42ch/spoke-connect/node";
import {
  NoiseHandshake,
  NoiseTransport,
  NoiseXX,
} from "@42ch/spoke-connect/noise";
import { connectRemoteAdapter } from "@42ch/spoke-connect/remote";

describe("package exports map", () => {
  it("resolves the root subpath by package name and exposes its entry points", () => {
    expect(typeof derivePeerIdFromEd25519Pubkey).toBe("function");
  });

  it("resolves the node subpath by package name and exposes its entry points", () => {
    expect(typeof connectClient).toBe("function");
  });

  it("resolves the noise subpath by package name and exposes its entry points", () => {
    expect(typeof NoiseXX).toBe("function");
    expect(typeof NoiseHandshake).toBe("function");
    expect(typeof NoiseTransport).toBe("function");
  });

  it("resolves the remote subpath by package name and exposes its entry points", () => {
    expect(typeof connectRemoteAdapter).toBe("function");
  });
});
