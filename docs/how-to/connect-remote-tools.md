---
title: Expose and invoke remote tools
---

# Expose and invoke remote tools

A connect session carries capability traffic in two directions: the host serves its `BaselinePorts` surface for the dialer to consume (the `port.*` direction), and the dialer can provide **tools** the host discovers from the authenticated manifest and reverse-invokes mid-orchestration. This guide covers the tool direction end to end — advertise, register, discover, invoke, feed — with snippets from the runnable demo (`examples/connect-demo/`, TypeScript) and the reference provider (`fixtures/toy-world/`, TypeScript + Rust) that they byte-match.

The story in one line: a client's manifest advertises tools in `tools[]`; the host lists them from the authenticated manifest, reverse-invokes one during an orchestration step, and feeds the result into a `BaselinePorts` call. The demo's tools are `tools.toy_world.roll_dice` (deterministic dice) and `tools.toy_world.lore_lookup` (read-only lore lookup) — the same frozen ids across the demo and the reference provider.

## 1. Advertise tools in the manifest

Three manifest fields describe the tools a dialer advertises for the host to discover and reverse-invoke over an established session:

| Field | Role |
|-------|------|
| `capabilities[]` | Lists the tool capability strings (e.g. `tools.toy_world.roll_dice`) so the pair negotiates them like any other capability |
| `namespaces[]` | Declares the namespaces the manifest owns; every declared tool's namespace must be listed |
| `tools[]` | Carries the full `ToolDescriptor` for each tool — capability id, wire op, description, argument/result subschemas, idempotency |

The demo client's manifest shows the shape:

```ts
export const DEMO_CLIENT_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-third-party-app",
  roles: ["input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
  ],
  namespaces: [DEMO_SCOPE_ID, "toy_world"],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};
```

A tool id follows the grammar `tools.<ns>.<tool_id>` (`^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$`): `tools.toy_world.roll_dice` names namespace `toy_world` and tool id `roll_dice`. The wire `op` of a tool equals its `capability_id` — the capability string is the op string.

`validateManifestTools` (from `@42ch/spoke-operations`) checks the manifest before it is used for discovery: each descriptor is well-formed, its `capability_id` appears in `capabilities[]`, its namespace is owned by `namespaces[]`, and tool ids are unique. The host runs the same check on the dialer's manifest at discovery time. See the [manifest `tools[]` field table](/reference/connect#manifest-tools-field-table) for the full descriptor fields.

## 2. Register handlers on a RemoteAdapter

A `RemoteAdapter` serves reverse invokes through registered handlers. Registration happens on the established adapter, right after the dial:

```ts
adapter.registerToolHandler(TOY_WORLD_ROLL_DICE_ID, rollDice);
adapter.registerToolHandler(
  TOY_WORLD_LORE_LOOKUP_ID,
  loreLookup(loreStore),
);
```

A handler receives the tool arguments object and resolves with a `SpokeResult`:

```ts
type ToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;
```

The demo's `rollDice` handler is deterministic — the seed is derived from the arguments, so the same arguments always produce the same rolls:

```ts
export function rollDice(args: Record<string, unknown>): Promise<ToolResult> {
  const count = args["count"];
  const sides = args["sides"] ?? 6;
  if (!isPositiveInteger(count)) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice count must be a positive integer", {
        field: "count",
      }),
    );
  }
  if (!isPositiveInteger(sides) || sides < 2) {
    return Promise.resolve(
      reject("INVALID_INPUT", "roll_dice sides must be an integer >= 2", {
        field: "sides",
      }),
    );
  }

  const random = mulberry32(fnv1a(`${count}:${sides}`));
  const rolls: number[] = [];
  for (let index = 0; index < count; index += 1) {
    rolls.push(1 + Math.floor(random() * sides));
  }
  const total = rolls.reduce((sum, roll) => sum + roll, 0);
  return Promise.resolve({ ok: true, value: { rolls, total } });
}
```

The tool's `ToolDescriptor` describes that ABI — the argument subschema, the result subschema, and advisory metadata:

```ts
export const ROLL_DICE_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: TOY_WORLD_ROLL_DICE_ID,
  op: TOY_WORLD_ROLL_DICE_ID,
  description:
    "Roll `count` dice with `sides` faces each. Deterministic: the same arguments always produce the same rolls (seeded from the arguments).",
  input: {
    type: "object",
    properties: {
      count: { type: "integer", minimum: 1 },
      sides: { type: "integer", minimum: 2 },
    },
    required: ["count"],
  },
  output: {
    type: "object",
    properties: {
      rolls: {
        type: "array",
        items: { type: "integer" },
      },
      total: { type: "integer" },
    },
    required: ["rolls", "total"],
  },
  idempotent: true,
};
```

Registration never mutates the manifest — descriptor truth for discovery stays in `tools[]` (sent through the hello). Registration is runtime state the validator cannot see: `validateManifestTools` checks the manifest's internal consistency only (descriptor well-formedness, capability membership, namespace ownership, id uniqueness), never the handler registry. A handler for a tool the manifest does not declare is a provider bug that surfaces at invoke time — the tool is never discoverable or negotiable, so the reverse invoke is denied (`op_unsupported` → `CAPABILITY_PORT_MISSING`), never a silent success. Registering a handler for a non-`tools.` id is a grammar error that throws. Duplicate registration for the same id overwrites the previous handler (last-wins).

The Rust provider mirrors the same surface with `register_tool_handler`, whose handler type is `Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>`:

```rust
use std::sync::Arc;
use serde_json::Value;

adapter.register_tool_handler(
    TOY_WORLD_ROLL_DICE_ID,
    Arc::new(|args: Value| Box::pin(async move { roll_dice(&args) })),
);
```

The reference provider's [`default_tool_handlers`](https://github.com/42ch-dev/spoke/blob/main/fixtures/toy-world/rust/src/toy_world_tools.rs) builds both handlers from this pattern — `roll_dice` and a `lore_lookup` bound to the adapter's store.

## 3. Discover tools from the authenticated manifest

The host learns what tools the dialer can serve from the authenticated manifest — the `host` embedded in the verified hello, cached on the responder as `remoteManifest`:

```ts
const manifest: HostCapabilityManifest = responder.remoteManifest;
const validated = validateManifestTools(manifest);
const discovered = listTools(manifest).map(
  (descriptor) => descriptor.capability_id,
);
```

`validateManifestTools` returns a `SpokeResult`; `listTools` returns the descriptors in declaration order. For the demo client this lists both tools:

```text
tools.toy_world.roll_dice
tools.toy_world.lore_lookup
```

Discovery is a property of the authenticated session, not of a separate advertisement step — the host reads the manifest it already verified at the handshake.

## 4. Reverse-invoke a tool mid-orchestration

The host reverse-invokes a discovered tool with `invokeTool` — a normal signed connect invoke in the reverse direction, carrying the tool arguments:

```ts
const result = await responder.invokeTool(
  TOY_WORLD_ROLL_DICE_ID,
  { ...ORCHESTRATION_ROLL_ARGS }, // { count: 2, sides: 6 }
);
```

The demo host runs this step inside its `putKnowledgeEntry` orchestration, right after the client's compass submission lands. On success the `result` is the handler's returned value — for 2d6 with the seeded algorithm it is always `{ rolls: [1, 2], total: 3 }`. The host then feeds that value into the engine as `demo-harbor/artifact/dice-roll`, a `BaselinePorts` step the client sees on its next `listKnowledgeEntries`.

A reverse invoke answers through the same deny vocabulary as any other op: invoking a tool the dialer did not list (and so did not negotiate) answers the wire code `op_unsupported`, which the library maps to a `CAPABILITY_PORT_MISSING` reject with `details.wire_code = "op_unsupported"`. The demo host records the deny and feeds nothing — the orchestration surfaces the denial instead of succeeding silently. The client's manifest must list the tool, and both sides must negotiate it: `tools.*` ops are dispatched only when the capability string itself is in the session's `negotiated_capabilities`. See [Reverse-invoke semantics](/reference/connect#reverse-invoke-semantics) in the wire reference.

The demo server wires the responder with its own manifest, the allowlist, and the orchestrator as `ports`:

```ts
void connectResponder({
  transport,
  identity: { seed: DEMO_SERVER_SEED },
  manifest: DEMO_SERVER_MANIFEST,
  allowlist: [DEMO_CLIENT_PEER_ID],
  peerKeys: {
    [DEMO_CLIENT_PEER_ID]: DEMO_CLIENT_PUBKEY,
  },
  ports: orchestrator,
}).then((responder) => {
  orchestrator.setResponder(responder);
});
```

The server's manifest declares the same tool ids the client serves, so both directions negotiate on the same session.

## 5. Run the demo

The demo runs over a real WebSocket in two terminals (build set in [the demo README](https://github.com/42ch-dev/spoke/blob/main/examples/connect-demo/README.md)):

```bash
node examples/connect-demo/server/dist/main.js --port 8787
```

```bash
node examples/connect-demo/client/dist/main.js --url ws://127.0.0.1:8787
```

The client prints each story step — dial, registered tools, the put during which the host reverse-invokes `roll_dice`, the resulting entries including `demo-harbor/artifact/dice-roll`, and the findings round-trip. The e2e gate boots the host on an ephemeral port and asserts the whole path, including the deterministic roll value and the capability-deny path:

```bash
pnpm -F @42ch/spoke-demo-client test
```

## Next steps

- [Connect wire reference](/reference/connect) — the manifest `tools[]` field table, the `tools.*` dispatch rule, and reverse-invoke semantics.
- [Connect architecture](/explanation/connect) — the bidirectional capability flow behind tools and ports.
- [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) — the copyable provider in TypeScript and Rust.
- [RemoteAdapter over a Transport](/how-to/connect-remote-adapter) — the dial and the `port.*` direction this guide builds on.
- [RemoteAdapter from native bindings](/how-to/remote-adapter-native-binding) — the same tool contract over the FFI surface (C#, Go, Kotlin, Python, Swift).
