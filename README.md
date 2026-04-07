# Mockpit

A high-performance HTTP mocking framework for Rust with template-based response generation, HAR recording, smart
consolidation, and GraphQL support.

## Features

- **Request Matching** - Express-style (`:id`), glob (`**/*.json`), regex, and exact URL patterns with priority-based
  selection
- **Template Responses** - Tera template engine with 115+ fake data functions for realistic responses
- **HAR Recording** - Record live HTTP traffic and convert to mock definitions
- **Smart Consolidation** - Detect patterns across recordings for 90%+ size reduction
- **GraphQL Support** - Auto-generate mocks from GraphQL schema introspection
- **Fake Data** - 115+ generators: names, emails, UUIDs, images, PDFs, and more
- **Stateful Mocking** - Thread-safe persistence store for multi-step workflows
- **Hot Reload** - File watcher with debouncing for live mock editing
- **HTTP API** - Axum-based management API for CRUD, bulk operations, and runtime control
- **CLI Tools** - Commands for creating, testing, validating, serving, and converting mocks

## Installation

### One-line install (macOS / Linux)

```sh
curl -sSf https://raw.githubusercontent.com/salamaashoush/mockpit/main/scripts/install.sh | sh
```

### Cargo install

```sh
cargo install mockpit-cli
```

### From source

```sh
git clone https://github.com/salamaashoush/mockpit
cd mockpit
cargo install --path crates/mockpit-cli
```

## Quick Start

### CLI

```sh
# Create a mock
mockpit mock create "/api/users/:id" -m GET -s 200 --template

# Serve mocks with hot reload
mockpit mock serve mocks/

# Generate fake data
mockpit fake data email --count 10

# Test mock matching
mockpit mock test -m GET /api/users/123 --render
```

### As a library

Add to your `Cargo.toml`:

```toml
[dependencies]
mockpit = { git = "https://github.com/salamaashoush/mockpit" }
```

### Basic Usage

```rust
use mockpit::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a mock registry and load mocks from a directory
    let registry = MockRegistry::new();
    registry.load_from_directory("mocks/").await?;

    // Create a matcher to evaluate incoming requests
    let matcher = MockMatcher::new(registry);

    // Match a request
    let ctx = RequestContext::new();
    if let Some(action) = matcher.find_match(&ctx).await {
        println!("Matched: {:?}", action);
    }
    Ok(())
}
```

### Mock File Format

```yaml
mocks:
- id: get-user
  priority: 100
  match:
    methods: ["GET"]
    url: "/api/users/:id"
  response:
    status: 200
    headers:
      content-type: "application/json"
    template: |
      {
        "id": "{{ captures.id }}",
        "name": "{{ fake_name() }}",
        "email": "{{ fake_email() }}"
      }
```

### With HTTP Server

```toml
[dependencies]
mockpit = { git = "https://github.com/salamaashoush/mockpit", features = ["server", "api"] }
```

### With GraphQL Support

```toml
[dependencies]
mockpit = { git = "https://github.com/salamaashoush/mockpit", features = ["graphql"] }
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `engine` | yes | Core mock engine (includes fake-data, type-detector, codegen) |
| `fake-data` | yes | 115+ fake data generators |
| `type-detector` | no | Semantic type detection from field names and JSON values |
| `codegen` | no | Template code generation from detected types |
| `graphql` | no | GraphQL schema introspection and mock generation |
| `server` | no | HTTP server with hot reload and graceful shutdown |
| `api` | no | Mock management HTTP API (axum router) |
| `schema` | no | JSON schema generation for editor validation |
| `full` | no | Enable everything |

## Crate Structure

```
mockpit          - Library: all core mock engine modules (types, config,
                   recorder, template, consolidator, engine, graphql,
                   type_detector, fake_data, codegen, core, server, api)
mockpit-cli      - CLI binary + lib (commands for mock management and
                   fake data generation)
```

## Documentation

- [Mock Engine](docs/MOCK_ENGINE.md) - Complete guide to the mocking system
- [Fake Data](docs/FAKE_DATA.md) - All 115+ generators with examples
- [GraphQL Mocks](docs/GRAPHQL_MOCKS.md) - Auto-generate mocks from GraphQL schemas
- [CLI Reference](docs/CLI_REFERENCE.md) - All CLI commands and flags

## License

MIT OR Apache-2.0
