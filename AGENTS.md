# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Mockpit is a high-performance Rust HTTP mocking framework. It provides request matching, template-based response
generation, HAR recording, smart consolidation, GraphQL mock generation, and fake data generators.

## Workspace Structure

Cargo workspace with 15 crates.

### Foundation Crates

- **mockpit** - Root facade crate with feature-gated re-exports of all sub-crates
- **mockpit-core** - Shared utilities: PersistenceStore (thread-safe KV store), levenshtein_distance
- **mockpit-types** - Core types: RequestContext, URL patterns, request/response matchers, body sources

### Mock System Crates

- **mockpit-config** - Mock configuration parsing (YAML/JSON), HAR file loading
- **mockpit-recorder** - HTTP request/response recording for mock generation
- **mockpit-codegen** - Template code generation from detected field types
- **mockpit-template** - Tera template rendering with 115+ fake data functions
- **mockpit-consolidator** - Smart mock consolidation with pattern detection (90%+ size reduction)
- **mockpit-engine** - Core engine: MockRegistry, MockMatcher, validation, scopes, call tracking

### Utility Crates

- **mockpit-type-detector** - Semantic type detection from field names and JSON values (40+ types)
- **mockpit-fake-data** - Fake data generators: names, emails, UUIDs, images, PDFs, etc.
- **mockpit-graphql** - GraphQL introspection parsing, SDL generation, and mock generation

### Integration Crates

- **mockpit-server** - HTTP server utilities: hot reload, graceful shutdown, file watcher, state management
- **mockpit-api** - Mock management HTTP API (axum router): CRUD, bulk ops, inspector, recording
- **mockpit-cli** - CLI commands for mock management and fake data generation

## Essential Commands

```bash
cargo build                              # Debug build
cargo nextest run                        # Run all tests (1153 tests)
cargo nextest run --package mockpit-engine  # Test specific crate
cargo check --workspace                  # Fast compile check
```

## Architecture

### Mock Request Flow

1. Request arrives -> `MockMatcher::find_match()`
2. URL pattern matching (Express, Glob, Regex, Exact) by priority
3. Header/query/body matching evaluation
4. Template rendering with fake data + captures
5. Response generation (inline body, template, file, or patch upstream)

### Mock Configuration Format

```yaml
mocks:
- id: get-user
  priority: 100
  match:
    methods: ["GET"]
    url: "/api/users/:id"
  response:
    status: 200
    template: '{"id": "{{ captures.id }}", "name": "{{ fake_name() }}"}'
```

### Feature Flags (root mockpit crate)

| Feature | Default | Description |
|---------|---------|-------------|
| `engine` | yes | Core mock engine |
| `fake-data` | yes | Fake data generators |
| `type-detector` | no | Semantic type detection |
| `codegen` | no | Template code generation |
| `graphql` | no | GraphQL introspection + mock generation |
| `server` | no | HTTP server with hot reload |
| `api` | no | Mock management HTTP API |
| `cli` | no | CLI commands |
| `schema` | no | JSON schema generation |
| `full` | no | Everything |

### API Route Prefix

The HTTP API uses `/__mockpit/` prefix by default. Consumers can customize via
`create_mock_router_with_prefix("/__custom_prefix")`.

## Code Standards

- Idiomatic Rust with zero-cost abstractions
- `anyhow::Result` for application code
- No `unsafe`, no `unwrap()` in production code
- All new code must include tests
- Run `cargo nextest run` before committing

## Testing

1153 tests across all crates. Uses cargo-nextest for parallel execution.

```bash
cargo nextest run                        # All tests
cargo nextest run -p mockpit-engine      # Specific crate
cargo nextest run test_name              # Specific test
```

## Mock Examples

Example mock files live in `mocks/examples/`:
- `advanced-matching/` - Header, body, URL, query param matching
- `stateful/` - Pagination, OAuth flow, file upload
- `templates-and-responses/` - Template features, delays, errors
- `graphql-examples.yaml` - GraphQL mock examples
- `flat-syntax-complete.yaml` - Complete mock syntax reference
