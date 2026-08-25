# Upgrading

Behaviour that changed between releases, and what to do about it. The
[CHANGELOG](CHANGELOG.md) lists every commit; this lists only the ones that can
break a working setup.

## 0.3.0 to 0.4.0

Six breaking changes, all of them in the state machines and the world doctor.
The short version: **nothing in the mock engine, the templates, the recorder or
the consolidator moved.** YAML mock files, HAR recordings and `setupServer`
handlers written against 0.3.0 all still work unchanged. What breaks is Rust
code that constructs the config structs field by field, or matches
exhaustively on the doctor's checks.

### Config structs gained fields

`MockConfig` gained `when`, `states` and `fire`; `MockCollectionConfig` gained
`machines`; `StateConfig` and `core::machine::State` gained `on` and `after`;
`CoverageReport` gained `unreached_states` and `untaken_edges`. Every one is
additive, and every one breaks a struct literal that listed all the fields.

```rust
// before
let config = MockConfig { id, priority, /* ... every field ... */ };

// after: let the defaults fill in what you did not set
let config = MockConfig { id, priority, ..MockConfig::default() };
```

`..Default::default()` is the fix, and it keeps working the next time a field
is added.

### `core::machine::Machine` no longer exposes its fields

`states` was public and is now private, and the new `on` is private too. A
machine is built by the config loader from a `machines:` block, which is how
every supported path already reached it; read its shape through the methods
rather than the fields.

### `doctor::Check` gained three variants

`UnreachableState`, `ShapeDisagreement` and `CrowdedDay`. A `match` over
`Check` needs a `_ => {}` arm, or arms for the three.

### `WorldConfig::field_rules` takes an argument

It now takes the collection's `machines`, because a field rule can name a
machine's state and resolving that needs the declarations. Pass `None` if you
declare no machines:

```rust
let rules = world_config.field_rules(None)?;
```

### Why the version jumped a minor rather than a patch

`cargo-semver-checks` reports these against the published 0.3.0, so the release
is 0.4.0 even though the surface most people use is unchanged. If you only load
mocks from files, nothing here applies to you.

## 0.2.0 to 0.3.0

Nine breaking changes, all of them in the entity world and the spec-derived
backends. The short version: **a world built from the same seed is not the same
world it was in 0.2.0.** Different instance counts, different ids, different
timestamps, and fields that used to always be present are now sometimes absent.

If you assert on generated values — golden files, recorded snapshots, hard-coded
ids — expect to re-record them once. Nothing here changes the shape of a
response, only the values in it and how many records there are.

### The world is a different size

`world.count` no longer applies one number to every entity. Counts derive from
each entity's place in the graph, so something at the root of a hierarchy gets
fewer instances than its children.

- Pin what you depend on with `world.counts: { User: 25 }` — a count stated per
  entity is taken literally and is unaffected.
- `world.scale` multiplies whatever the defaults resolve to, for a bigger or
  smaller world without naming every entity.
- `count: 0` now means zero. It previously produced one instance of everything,
  because the clamp that stops a scale factor rounding a real count away was
  also rounding an explicit zero up. An empty world you fill through the API is
  now reachable.

### Ids and timestamps changed, and now agree with each other

Records have a history. Creation times come from a monotone function of an
instance's ordinal, with weekly and daily structure, and ids sort in the same
order as the times beside them.

- Hard-coded ids in tests will not resolve. Read one from a list response
  instead of writing it down.
- The window is anchored relative to now rather than to a fixed range, so
  timestamps stay plausible as time passes instead of receding into the past.

### Optional fields are sometimes absent, and sometimes null

`required` and `nullable` were one flag and are now two, because they are two
facts. An optional property is now genuinely missing from some records; a
nullable one is genuinely null.

- A client that assumed every declared field is present on every record will
  see gaps. That is what the schema said all along.
- Filters treat an absent field as matching only `!=`, so a query filtering on
  a field some records omit returns fewer of them.

### Relations answer from one mechanism

`folder.parent`, `folder.children` and `folder.children_count` could each
answer from a different mechanism and disagree. They now agree.

- Any test that depended on the old disagreement — most likely a count that did
  not match the collection beside it — will change.
- Hierarchies have roots. A self-relation is laid out in levels, so a parent is
  always above its child and a breadcrumb walk terminates. Previously about a
  third of records in a hierarchy were their own parent.
- Deleting cascades to the bottom rather than one level down.

### `viewer`, `me` and `GET /me` need to be told who is asking

A root field returning a single entity with no argument used to answer with
record zero, for every caller, token or not.

- Name the entity a credential is an instance of: `world.viewer: User`.
- Which instance is derived from the credential, so one token is the same person
  on every request and across a restart, and two tokens are two people.
- Without `world.viewer`, these endpoints answer with an error rather than
  quietly handing back the first record.

### Writes actually write

Mutations whose key arrives inside an input object — `deleteUser(input: { id })`,
the Relay convention — used to classify as unclassified and answer with a
well-formed payload while changing nothing.

- Those mutations now apply. A test that passed because a delete did nothing
  will start failing, and it was passing for the wrong reason.
- The same is true of a mutation whose payload carries no entity at all
  (`DeleteUserResponse { errors }`): the entity comes from the mutation's name.
- `PUT`/replace really replaces. A replacement that omits a field no longer gets
  that field back from the derived layer on the next read.

### REST answers with the shape its own document declared

Where one entity is described by both a GraphQL schema and an OpenAPI document,
the store merges the fields but each surface now answers in its own contract.

- A REST response no longer carries properties its document never declared.
  If you were relying on a field that only the GraphQL schema mentions, declare
  it in the document too.
- GraphQL is unaffected: a selection set already projects.

### Non-null lookups that miss are errors

A non-null field whose record does not exist used to surface the executor's
`internal: non-null types require a return value`. It now answers with an error
naming the entity and the key. Nullable fields still answer null.

## New in 0.3.0, worth knowing about

- `world.persistence: state.json` keeps a world's writes across a restart. What
  is written is the delta — the writes laid over the seed — not the entities,
  and it is written when the process stops rather than on every mutation. A run
  killed outright starts from its seed again.
- `ferrimock world doctor` lints a generated world for the things that give a
  mock away: uniform enums, fields that are never absent, arrays of constant
  length, numbers whose support never moves, ids that do not order with time.
- `ferrimock world explain` reports the entities, the rule and confidence behind
  every inferred relation, and which operations are answered from the world.
