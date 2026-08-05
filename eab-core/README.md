# Integrating `eab-core` into a Rust game

`eab-core` provides transport-neutral achievement definitions, offline event
evaluation, and immutable local achievement records. A game can use it without
running an EAB server and add online synchronization in a later release.

The crate does not depend on Actix, qcoin, `loadngo`, or an online identity.
It contains no trusted-service credentials or authoritative account mutation.

## Current capabilities

- one-time achievements
- event-key and numeric-threshold evaluation
- once-per-local-player deduplication across definition versions
- optional evidence and explicit online-claim readiness
- stable local award and future online claim identities
- in-memory storage for tests
- durable, single-writer JSON-lines reference storage
- a storage trait for integration with an existing game save system
- SHA-256 definition digests and local record integrity checks

Repeatable achievements, accumulated progress managed by the core, crash-tail
recovery, and a persistent synchronization outbox are not implemented yet.

## 1. Add the dependency

For a game using a local checkout:

```toml
[dependencies]
eab-core = { path = "../entitlement-achievement-blockchain/eab-core" }
```

For a Git dependency, pin a reviewed revision rather than following a moving
branch:

```toml
[dependencies]
eab-core = { git = "https://github.com/jaylauffer/entitlement-achievement-blockchain", rev = "<commit>" }
```

The repository does not currently define a crates.io release workflow. Treat
the schema and API as pre-release and pin the exact source revision used by a
shipped game.

## 2. Package achievement definitions

Definitions are game data. Each definition contains:

- namespace identity: developer, game, achievement id, and version
- presentation: name, description, and category
- policy: visibility, repeatability, and issuance mode
- accomplishment rule: event key, threshold, and evidence requirement

The same versioned definition should be registered with the future online EAB
authority. The local record stores its deterministic `definition_digest`, so a
changed or mismatched online definition produces a conflict rather than being
silently accepted.

Definitions implement Serde serialization and deserialization. A game may
construct them in Rust, load them from its normal data format, or generate them
during its asset build. Concrete product achievements should not be added to
the shared `eab-core` crate.

```rust
use eab_core::{
    AchievementAccomplishment, AchievementDefinition,
    AchievementIssuanceMode, AchievementRepeatability,
    AchievementVisibility,
};

fn first_flight_definition() -> AchievementDefinition {
    AchievementDefinition::new(
        "pudding",
        "solo-flight",
        "first-flight",
        1,
        "First Flight",
        "Complete a successful flight",
    )
    .with_category("progression")
    .with_policy(
        AchievementVisibility::Private,
        AchievementRepeatability::OncePerPlayer,
        AchievementIssuanceMode::DirectAwardOrClaimReview,
    )
    .with_accomplishment(AchievementAccomplishment {
        summary: "Complete one successful flight".into(),
        event_key: Some("flight_completed".into()),
        threshold: Some(1),
        requires_evidence: false,
    })
}
```

## 3. Define stable game identifiers

The game supplies an `OfflineAchievementContext` when evaluating an event:

| Field | Expected lifetime |
| --- | --- |
| `local_player_id` | Stable identity of a local profile or player slot. |
| `save_id` | Stable identity stored inside that save and preserved by copies. |
| `installation_id` | Random, pseudonymous installation identity; do not use a hardware fingerprint. |
| `session_id` | New random identity for each gameplay session. |
| `client_sequence` | Monotonically increasing event/order value within the session. |
| `game_build` | Shipped build or content version that evaluated the event. |

Persist installation and save identifiers before recording achievements. Do
not generate new values on every call. A copied save should retain its existing
save, award, and claim identities.

`earned_at_local` is useful presentation and provenance data, but the device
clock is not authoritative. Session identity and sequence provide additional
ordering context while still remaining client-originated.

## 4. Open storage during game startup

The reference file store reloads and verifies existing records when opened:

```rust,no_run
use eab_core::FileOfflineAchievementStorage;

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let achievements_path = "<private-game-data>/offline-achievements.jsonl";
let mut achievement_storage =
    FileOfflineAchievementStorage::open(achievements_path)?;
# let _ = &mut achievement_storage;
# Ok(())
# }
```

Use the platform's private application-data or save-data directory. The file
store is append-only and single-writer: keep one mutable owner and route game
events to it rather than opening it concurrently from multiple systems.

Opening fails closed if a record is malformed, duplicated, or fails its
integrity check. The current implementation does not repair a partially
written final line automatically. A production game should decide whether to
restore a known-good backup, offer recovery, or use its transactional save
database instead.

## 5. Record an achievement event

Call `record_offline_achievement` only after the game itself has established
the relevant gameplay fact:

```rust,no_run
use eab_core::{
    record_offline_achievement, FileOfflineAchievementStorage,
    OfflineAchievementContext, OfflineAchievementEvent, OfflineAwardOutcome,
};

# fn first_flight_definition() -> eab_core::AchievementDefinition {
#     eab_core::AchievementDefinition::new("pudding", "solo-flight", "first-flight", 1, "First Flight", "Complete a successful flight")
#       .with_accomplishment(eab_core::AchievementAccomplishment { summary: "Complete one successful flight".into(), event_key: Some("flight_completed".into()), threshold: Some(1), requires_evidence: false })
# }
# fn example(storage: &mut FileOfflineAchievementStorage) -> Result<(), Box<dyn std::error::Error>> {
let event = OfflineAchievementEvent {
    event_key: "flight_completed".into(),
    value: 1,
    occurred_at: "2026-08-05T12:00:00Z".into(),
    evidence: None,
};
let context = OfflineAchievementContext {
    local_player_id: "player-slot-1".into(),
    save_id: "save-1".into(),
    installation_id: "installation-1".into(),
    session_id: "session-1".into(),
    client_sequence: 7,
    game_build: "1.0.0".into(),
};

match record_offline_achievement(
    storage,
    &first_flight_definition(),
    &event,
    &context,
)? {
    OfflineAwardOutcome::Awarded(record) => {
        // Persistence succeeded. It is now safe to show the unlock.
        println!("achievement unlocked: {}", record.achievement_id);
    }
    OfflineAwardOutcome::AlreadyAwarded(record) => {
        // Idempotent replay: use the original record and claim id.
        println!("already unlocked as {}", record.local_award_id);
    }
    OfflineAwardOutcome::NoMatchingEvent => {}
    OfflineAwardOutcome::ThresholdNotMet { observed, required } => {
        println!("achievement progress: {observed}/{required}");
    }
}
# Ok(())
# }
```

Do not display a durable unlock before this function returns `Awarded`. A
storage error means durable commit was not confirmed and must follow the
game's recovery policy.

The core compares the supplied value with the definition threshold; it does
not currently aggregate progress across calls. The game may submit an already
computed cumulative value if that matches its design.

## 6. Restore achievement UI after restart

Import `OfflineAchievementStorage` to enumerate successfully loaded records:

```rust,no_run
use eab_core::{
    FileOfflineAchievementStorage, OfflineAchievementStorage,
};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let storage =
    FileOfflineAchievementStorage::open("offline-achievements.jsonl")?;

for record in storage.records() {
    println!(
        "{} earned at {}",
        record.achievement_id,
        record.earned_at_local,
    );
}
# Ok(())
# }
```

The record is the durable game-scoped receipt. Presentation text may come from
the game's current localized definition assets rather than being duplicated
inside every record.

## 7. Choose a persistence integration

`MemoryOfflineAchievementStorage` is intended for tests and transient tools.

`FileOfflineAchievementStorage` is a reference implementation. It:

- appends one immutable JSON record per line
- flushes and calls `sync_data()` before returning success
- reloads records after restart
- rejects duplicate local-award or claim identities
- verifies every record's integrity hash on load

For a production game, implementing `OfflineAchievementStorage` over the
existing save system is often preferable:

```rust
use eab_core::{
    OfflineAchievementError, OfflineAchievementRecord,
    OfflineAchievementStorage,
};

struct GameSaveAchievementStorage {
    records: Vec<OfflineAchievementRecord>,
    // Database, save transaction, or engine persistence handle.
}

impl OfflineAchievementStorage for GameSaveAchievementStorage {
    fn records(&self) -> &[OfflineAchievementRecord] {
        &self.records
    }

    fn append(
        &mut self,
        record: &OfflineAchievementRecord,
    ) -> Result<(), OfflineAchievementError> {
        // 1. Commit the record in the game's durable save transaction.
        // 2. Only after that succeeds, append record.clone() to self.records.
        // 3. Map persistence failures to OfflineAchievementError::Storage.
        todo!("integrate the game's persistence layer")
    }
}
```

The storage implementation must reject or safely handle duplicate
`local_award_id` and `claim_id` values and must not report success before the
durable transaction commits. If achievement and game progress must never
diverge, commit them in the same save transaction.

## 8. Understand local security

Each record has a SHA-256 `local_record_hash`, and each definition has a
SHA-256 digest. These checks detect corruption and unsophisticated editing.
They are not an anti-cheat boundary: a player who controls the executable can
recalculate an unkeyed hash.

Recommended baseline protections:

- store records in the platform's private application-data directory
- minimize evidence and other sensitive local data
- use platform-backed encryption when local evidence requires confidentiality
- use the game's transactional save and backup/recovery facilities
- use random pseudonymous installation ids, never hardware fingerprints
- treat local clocks, sequences, evidence, and signatures as provenance rather
  than proof of player honesty

A device-generated signing key held in Keychain, Android Keystore, TPM, or
Secure Enclave can improve installation continuity and resistance to casual
external editing. It cannot make a player-controlled offline game authoritative
for competitive or economic rewards. Those require stronger server-observed,
platform-attested, or reviewed provenance.

## 9. Preserve records for transport later

Transport is optional. Every offline award receives its final `claim_id` when
created. Never replace that id during upload or retry.

When online synchronization is added:

1. select records whose `claim_readiness` is `Ready`
2. wrap the complete record in `EabClaimEnvelope`
3. authenticate and bind the transport separately to the intended EAB account
4. submit idempotently using the existing claim id
5. store pending/submitted/acknowledged synchronization state separately from
   the immutable achievement record

```rust,no_run
use eab_core::{
    EabClaimEnvelope, FileOfflineAchievementStorage,
    OfflineAchievementStorage, OfflineClaimReadiness,
};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let storage =
    FileOfflineAchievementStorage::open("offline-achievements.jsonl")?;

for record in storage.records() {
    if record.claim_readiness == OfflineClaimReadiness::Ready {
        let envelope = EabClaimEnvelope::try_from(record)?;
        // A future HTTP or authenticated-unicast adapter sends `envelope`.
        // Account identity and credentials do not belong inside the record.
        let _ = envelope;
    }
}
# Ok(())
# }
```

A local achievement remains a valid game-scoped acknowledgement even if a
future authority rejects or cannot accept it online. The game should present
local and account acknowledgement states separately.

## 10. Test the game integration

At minimum, test that:

- a matching event persists exactly one record
- a below-threshold event writes nothing
- repeated events return `AlreadyAwarded` with the original claim id
- restarting the game reloads the same record
- a storage error does not display or commit an unlock
- malformed or modified persisted data follows the game's recovery policy
- copied saves preserve existing award and claim identities
- claim-ready records survive until a later synchronization attempt

Run the included two-launch example with an explicit temporary path:

```sh
cargo run -p eab-core --example offline_game -- /tmp/eab-offline-example.jsonl
```

The first simulated launch records the achievement. The second reopens the
file, repeats the event, and receives the original record as `AlreadyAwarded`.

## Further design documentation

- [Stand-alone offline achievement support](../docs/STANDALONE_OFFLINE_ACHIEVEMENT_SUPPORT.md)
- [Canonical claim transport](../docs/EAB_CLAIM_TRANSPORT.md)
- [Authorization and offline claims](../docs/AUTHORIZATION_AND_OFFLINE_CLAIMS.md)
