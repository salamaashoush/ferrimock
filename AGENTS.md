# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Mockpit is a high-performance Rust HTTP mocking framework. It provides request matching, template-based response
generation, HAR recording, smart consolidation, GraphQL mock generation, and fake data generators.

## Workspace Structure

Cargo workspace with 2 crates.

### mockpit (library)

All core mock engine logic in one publishable crate. Modules:

- `core` - Shared utilities: PersistenceStore (thread-safe KV store), levenshtein_distance
- `types` - Core types: RequestContext, URL patterns, request/response matchers, body sources
- `config` - Mock configuration parsing (YAML/JSON), HAR file loading
- `recorder` - HTTP request/response recording for mock generation
- `codegen` - Template code generation from detected field types
- `template` - Tera template rendering with 115+ fake data functions
- `consolidator` - Smart mock consolidation with pattern detection (90%+ size reduction)
- `engine` - Core engine: MockRegistry, MockMatcher, validation, scopes, call tracking
- `type_detector` - Semantic type detection from field names and JSON values (40+ types)
- `fake_data` - Fake data generators: names, emails, UUIDs, images, PDFs, etc.
- `graphql` - GraphQL introspection parsing, SDL generation, and mock generation
- `server` - HTTP server utilities: hot reload, graceful shutdown, file watcher, state management
- `api` - Mock management HTTP API (axum router): CRUD, bulk ops, inspector, recording

### mockpit-cli (lib + binary)

CLI binary and command implementations. Has both `[[bin]]` and `[lib]` sections.

- `commands/` - Mock management and fake data CLI commands
- `main.rs` - Binary entry point
- lib exports: `MockCommand`, `FakeCommand`, `execute`, `fake`

## Essential Commands

```bash
cargo build                              # Debug build
cargo nextest run                        # Run all tests (1139 tests)
cargo nextest run --package mockpit      # Test mockpit library
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

### Feature Flags (mockpit crate)

| Feature | Default | Description |
|---------|---------|-------------|
| `engine` | yes | Core mock engine (includes fake-data, type-detector, codegen) |
| `fake-data` | yes | Fake data generators |
| `type-detector` | no | Semantic type detection |
| `codegen` | no | Template code generation |
| `graphql` | no | GraphQL introspection + mock generation |
| `server` | no | HTTP server with hot reload |
| `api` | no | Mock management HTTP API |
| `schema` | no | JSON schema generation |
| `full` | no | Everything |

### Extension APIs

Embedders can extend mockpit without modifying its source:

- `mockpit::template::register_template_function(name, closure)` - custom template functions
- `mockpit::config::DomainFilter` trait on `HarLoadOptions` - domain filtering for HAR loading
- `mockpit::type_detector::register_url_classifier(closure)` - custom download URL detection
- `mockpit::codegen::register_file_object_detector(detector)` - file object detection in responses
- `mockpit::consolidator::register_path_normalizer(closure)` - custom URL path normalization
- `mockpit::core::set_app_name(name)` - app identity for HAR exports

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

1139 tests. Uses cargo-nextest for parallel execution.

```bash
cargo nextest run                        # All tests
cargo nextest run --package mockpit      # Library tests
cargo nextest run test_name              # Specific test
```

## Mock Examples

Example mock files live in `mocks/examples/`:
- `advanced-matching/` - Header, body, URL, query param matching
- `stateful/` - Pagination, OAuth flow, file upload
- `templates-and-responses/` - Template features, delays, errors
- `graphql-examples.yaml` - GraphQL mock examples
- `flat-syntax-complete.yaml` - Complete mock syntax reference
