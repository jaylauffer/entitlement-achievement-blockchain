# EAB Transport Design Goals

Purpose: state the intended transport/runtime direction for
`entitlement-achievement-blockchain` clearly enough that future work does not
accidentally optimize around the current HTTP adapter as if it were the final
architecture.

This note is about EAB transport and runtime posture, not player-facing reward
semantics.

## Core direction

The intended direction is:

- `EAB core/runtime` on `loadngo-proactor`
- `EAB node discovery/service plane` on `loadngo/network`
- `IPv6 multicast` for low-friction node discovery and presence
- `unicast` for deterministic follow-up and state exchange
- `HTTP` retained as a current compatibility/client/trusted-service adapter

The key distinction is:

- HTTP is important **today** because it is the current public and
  trusted-service mutation surface
- HTTP is **not** the architectural destination for EAB node-to-node behavior

## Final target shape

The final target for the EAB node plane should be:

### 1. IPv6 multicast for discovery and announcements

Use multicast for:

- `PresenceAnnounce`
- bootstrap node discovery
- low-rate service advertisements

Do **not** use multicast for:

- heavy payload replication
- durable receipt exchange
- reward issuance authority
- anything that depends on reliable delivery

The multicast plane should remain intentionally small and low amplification.

### 2. Unicast for deterministic work

Use unicast for:

- `NodeInfo`
- `StatusRequest` / `StatusResponse`
- anchor lifecycle status
- future receipt fetch / reconciliation
- any later node-to-node coordination that requires clear request/response
  semantics

This is where real EAB node behavior should live.

### 3. HTTP as an adapter, not the node protocol

HTTP remains useful for:

- current game/service integration
- trusted-service operations
- compatibility during transition
- debugging and operator inspection

But the design goal is **not**:

- "make HTTP more central"

The design goal is:

- "make HTTP one adapter onto a loadngo-owned core"

## Why the HTTP reward path still matters today

The current authoritative reward flow is exercised through HTTP because that is
where the real mutation path exists today.

So when tests mention the "HTTP reward path", that should be interpreted as:

- "exercise the real current authoritative mutation path"

and **not** as:

- "the future EAB architecture depends on HTTP reward transport"

The distinction matters.

## Minimal-configuration lab target

For the 3-device lab, the target operator experience should be:

- minimal per-node config
- embedded IPv6 multicast bootstrap defaults
- one shared understanding of cluster identity
- direct unicast follow-up once peers are discovered

That means the preferred path is:

1. start the node
2. join the embedded IPv6 multicast group
3. announce presence
4. learn peers
5. switch to direct unicast exchange for deterministic work

Explicit interface pinning or static peers should remain escape hatches, not the
normal path.

## Reward anchoring implications

For qcoin-backed reward anchoring, this implies:

- local EAB authority remains local-first
- qcoin anchoring runs as background runtime work
- anchor lifecycle should be observable over the loadngo service plane
- qcoin inclusion state should not depend on HTTP request ownership

So the long-term target is:

- reward issued by EAB core
- anchor progressed by loadngo-owned runtime
- status and reconciliation visible over the multicast/unicast node plane

not:

- reward correctness tied to an HTTP request thread

## What this means for current testing

Near-term testing should be split into two categories:

### 1. Current-path correctness tests

These may still use HTTP because that is the current authoritative mutation
surface.

Examples:

- authoritative achievement award
- authoritative entitlement grant
- local receipt generation
- qcoin anchor enqueueing from the real current path

### 2. Target-architecture tests

These should validate the loadngo node plane directly.

Examples:

- multicast discovery
- direct unicast status exchange
- anchor lifecycle visibility over node status
- multi-node EAB status consistency

The project should avoid confusing category `1` with the final destination.

## Practical rule for future work

When choosing the next EAB task:

- if it makes the loadngo runtime/node plane more truthful or useful, it is
  aligned with the target
- if it merely strengthens HTTP ownership without helping the loadngo-owned
  core, it is probably not the right next move

## Relationship to other notes

This note complements:

- [ACHIEVEMENT_MODEL.md](ACHIEVEMENT_MODEL.md)
- [LOADNGO_RUNTIME_MIGRATION.md](LOADNGO_RUNTIME_MIGRATION.md)
- [QCOIN_ANCHOR_ACCEPTANCE_GATE.md](QCOIN_ANCHOR_ACCEPTANCE_GATE.md)
- [QCOIN_REWARD_ANCHOR_DISCOVERY.md](QCOIN_REWARD_ANCHOR_DISCOVERY.md)
