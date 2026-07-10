---
sidebar_label: GraphQL Mocks
section: Tools
order: 3
---

# GraphQL Mock Generation

Auto-generate mocks from GraphQL schema introspection with type-aware fake data.

## Quick Start

```bash
# Generate from schema
ferrimock graphql mock /graphql --no-auth -o mocks.yaml

# Use with proxy
ferrimock proxy --mock --mock-file mocks.yaml
```

Generated mock example:

```yaml
mocks:
- id: query-currentUser
  priority: 100
  match:
    methods: ["POST"]
    url: "/graphql"
    graphql:
      operation: currentUser
  return:
    status: 200
    body: |
      {
        "data": {
          "currentUser": {
            "id": "{{ uuid() }}",
            "name": "{{ fake_name() }}",
            "email": "{{ fake_email() }}"
          }
        }
      }
```

## How It Works

1. **Introspect** - Fetch GraphQL schema via introspection query
2. **Analyze** - Parse types, queries, mutations, subscriptions
3. **Map** - Match GraphQL types to fake data generators
4. **Generate** - Create mock configs with Tera templates

## Basic Usage

```bash
# Custom endpoint
ferrimock graphql mock /custom/graphql -o mocks.yaml

# Different format
ferrimock graphql mock /graphql -o mocks.json  # JSON
ferrimock graphql mock /graphql -o mocks.yaml  # YAML
```

## Operation Filtering

```bash
# Queries only
ferrimock graphql mock /graphql --no-mutations --no-subscriptions -o queries.yaml

# Mutations only
ferrimock graphql mock /graphql --no-queries --mutations -o mutations.yaml

# All operations (queries + mutations + subscriptions)
ferrimock graphql mock /graphql --subscriptions -o all.yaml
```

| Flag                 | Default | Description                     |
| -------------------- | ------- | ------------------------------- |
| `--queries`          | true    | Include query operations        |
| `--mutations`        | true    | Include mutation operations     |
| `--subscriptions`    | false   | Include subscription operations |
| `--no-queries`       | -       | Exclude queries                 |
| `--no-mutations`     | -       | Exclude mutations               |
| `--no-subscriptions` | -       | Exclude subscriptions (default) |

## Type Mappings

Override default GraphQL type to fake data mappings.

### Create Mappings File

```json
{
  "scalars": {
    "DateTime": "\"{{ now() }}\"",
    "Email": "\"{{ fake_email() }}\"",
    "UUID": "\"{{ uuid() }}\"",
    "Money": "{{ get_random(start=1.0, end=999.99) }}"
  }
}
```

Or YAML:

```yaml
scalars:
  DateTime: '"{{ now() }}"'
  Email: '"{{ fake_email() }}"'
  UUID: '"{{ uuid() }}"'
  Money: "{{ get_random(start=1.0, end=999.99) }}"
```

### Use Mappings

```bash
ferrimock graphql mock /graphql \
  --type-mappings custom-types.json \
  -o mocks.yaml
```

### Built-in Mappings

| GraphQL Type  | Template                                  | Output        |
| ------------- | ----------------------------------------- | ------------- |
| `ID`          | `{{ uuid() }}`                            | UUID v4       |
| `String`      | `{{ fake_sentence(word_count=8) }}`       | Sentence      |
| `Int`         | `{{ get_random(start=1, end=1000) }}`     | Integer       |
| `Float`       | `{{ get_random(start=0.0, end=1000.0) }}` | Decimal       |
| `Boolean`     | `{{ fake_boolean() }}`                    | true/false    |
| `DateTime`    | `{{ now() }}`                             | ISO timestamp |
| `Date`        | `{{ fake_iso_date() }}`                   | ISO date      |
| `Email`       | `{{ fake_email() }}`                      | Email address |
| `URL`         | `{{ fake_url() }}`                        | HTTP URL      |
| `UUID`        | `{{ uuid() }}`                            | UUID v4       |
| `PhoneNumber` | `{{ fake_phone() }}`                      | Phone number  |

Custom mappings override built-in defaults.

## Generation Options

```bash
ferrimock graphql mock /graphql \
  --priority 150 \                    # Mock priority (default: 100)
  --list-length 5 \                   # Array length (default: 3)
  --max-depth 4 \                     # Nesting depth (default: 5)
  --include-deprecated \              # Include deprecated fields
  --generate-variants \               # Generate multiple scenarios
  -o mocks.yaml
```

| Option                 | Default | Description                                         |
| ---------------------- | ------- | --------------------------------------------------- |
| `--priority <N>`       | 100     | Base priority for generated mocks                   |
| `--list-length <N>`    | 3       | Default array/list length                           |
| `--max-depth <N>`      | 5       | Maximum nesting depth (prevents infinite recursion) |
| `--include-deprecated` | false   | Include deprecated schema fields                    |
| `--generate-variants`  | false   | Generate success/error/empty variants               |

## Smart Features

### Variable Injection

Automatically injects GraphQL variables into mock responses:

```graphql
query GetUser($id: ID!) {
  user(id: $id) {
    id
    name
  }
}
```

Generated mock:

```yaml
mocks:
- id: query-user
  match:
    graphql:
      operation: GetUser
  return:
    body: |
      {
        "data": {
          "user": {
            "id": "{{ body_json.variables.id }}",
            "name": "{{ fake_name() }}"
          }
        }
      }
```

### Pagination Support

Detects pagination variables and adjusts array lengths:

```graphql
query ListUsers($limit: Int, $offset: Int) {
  users(limit: $limit, offset: $offset) {
    id
    name
  }
}
```

Generated:

```yaml
return:
  body: |
    {
      "data": {
        "users": {% set __array_length = body_json.variables.limit | default(value=3) %}
        [
          {% for i in range(start=0, end=__array_length) %}
          {"id": "{{ uuid() }}", "name": "{{ fake_name() }}"}
          {% if not loop.last %},{% endif %}
          {% endfor %}
        ]
      }
    }
```

### Type Detection

Uses semantic field name detection for better fake data:

| Field Name               | Detected Type | Generator      |
| ------------------------ | ------------- | -------------- |
| `email`, `emailAddress`  | Email         | `fake_email()` |
| `createdAt`, `updatedAt` | DateTime      | `now()`        |
| `firstName`, `lastName`  | Name          | `fake_name()`  |
| `phoneNumber`, `phone`   | Phone         | `fake_phone()` |
| `userId`, `postId`       | ID            | `uuid()`       |

### Union & Interface Support

Generates conditional templates with `__typename`:

```yaml
return:
  body: |
    {
      "data": {
        "search": {% set __union_type = ["User", "Post"] | random_choice %}
        {% if __union_type == "User" %}
        {
          "__typename": "User",
          "id": "{{ uuid() }}",
          "name": "{{ fake_name() }}"
        }
        {% elif __union_type == "Post" %}
        {
          "__typename": "Post",
          "id": "{{ uuid() }}",
          "title": "{{ fake_sentence() }}"
        }
        {% endif %}
      }
    }
```

## Authentication

### Authenticated Endpoints

```bash
# With bearer token header
ferrimock graphql mock https://api.example.com/graphql \
  -H "Authorization: Bearer <token>" \
  -o mocks.yaml
```

### Public Endpoints

```bash
ferrimock graphql mock https://api.example.com/graphql --no-auth -o mocks.yaml
```

### Via Proxy

```bash
# Start proxy first
ferrimock proxy --mock

# Generate via proxy (proxy handles auth)
ferrimock graphql mock http://localhost:3003/graphql \
  --proxied \
  -o mocks.yaml
```

## Integration

### Load Mocks

```bash
# Direct file
ferrimock proxy --mock --mock-file graphql-mocks.yaml

# Auto-load from collections/
mv graphql-mocks.yaml mocks/collections/
ferrimock proxy --mock
```

### Combine with Other Mocks

```yaml
# mocks/collections/main.yaml

# Include GraphQL mocks
include:
- path: "graphql-mocks.yaml"

# Add custom overrides
mocks:
- id: query-currentUser
  priority: 200 # Higher priority
  return:
    body: '{"data": {"currentUser": {"id": "admin-123", "name": "Admin"}}}'
```

### Test Generated Mocks

```bash
# Start proxy
ferrimock proxy --mock --mock-file graphql-mocks.yaml

# Test query
curl http://localhost:3003/graphql \
  -X POST \
  -H "Content-Type: application/json" \
  -d '{"query": "query { currentUser { id name } }"}'
```

## Example Schema

GraphQL schema:

```graphql
type Query {
  currentUser: User
  user(id: ID!): User
  users(limit: Int): [User!]!
}

type Mutation {
  updateUser(id: ID!, input: UserInput!): User
}

type User {
  id: ID!
  name: String!
  email: String
  createdAt: DateTime!
}

input UserInput {
  name: String
  email: String
}

scalar DateTime
```

Generated YAML (excerpt):

```yaml
mocks:
- id: query-currentUser
  priority: 100
  description: "GraphQL Query: currentUser"
  match:
    methods: ["POST"]
    url: "/graphql"
    graphql:
      operation: currentUser
      query: currentUser
  return:
    status: 200
    headers:
      Content-Type: "application/json"
    body: |
      {
        "data": {
          "currentUser": {
            "id": "{{ uuid() }}",
            "name": "{{ fake_name() }}",
            "email": "{{ fake_email() }}",
            "createdAt": "{{ now() }}"
          }
        }
      }

- id: query-user
  priority: 100
  match:
    methods: ["POST"]
    url: "/graphql"
    graphql:
      operation: user
      query: user
  return:
    status: 200
    body: |
      {
        "data": {
          "user": {
            "id": "{{ body_json.variables.id }}",
            "name": "{{ fake_name() }}",
            "email": "{{ fake_email() }}",
            "createdAt": "{{ now() }}"
          }
        }
      }

- id: mutation-updateUser
  priority: 100
  match:
    methods: ["POST"]
    url: "/graphql"
    graphql:
      operation: updateUser
      mutation: updateUser
  return:
    status: 200
    body: |
      {
        "data": {
          "updateUser": {
            "id": "{{ body_json.variables.id }}",
            "name": "{{ body_json.variables.input.name }}",
            "email": "{{ body_json.variables.input.email }}",
            "createdAt": "{{ now() }}"
          }
        }
      }
```

## Customization

Edit generated mocks for specific behavior:

```yaml
mocks:
- id: query-currentUser
  enabled: true
  priority: 200 # Higher priority for custom version
  match:
    graphql:
      operation: currentUser
      variables:
        includeDetails: "true"
  return:
    status: 200
    body: |
      {
        "data": {
          "currentUser": {
            "id": "admin-123",
            "name": "Admin User",
            "email": "admin@example.com",
            "role": "admin",
            "permissions": ["read", "write", "admin"]
          }
        }
      }
```

## CLI Reference

```bash
ferrimock graphql mock [ENDPOINT] [OPTIONS]
```

| Option                   | Default    | Description                      |
| ------------------------ | ---------- | -------------------------------- |
| `[ENDPOINT]`             | `/graphql` | GraphQL endpoint URL or path     |
| `-o, --output <FILE>`    | -          | Output file path (required)      |
| `-f, --format <FORMAT>`  | yaml       | Output format: yaml, json        |
| `--queries`              | true       | Include queries                  |
| `--no-queries`           | -          | Exclude queries                  |
| `--mutations`            | true       | Include mutations                |
| `--no-mutations`         | -          | Exclude mutations                |
| `--subscriptions`        | false      | Include subscriptions            |
| `--type-mappings <FILE>` | -          | Custom type mappings (JSON/YAML) |
| `--priority <N>`         | 100        | Base priority for mocks          |
| `--list-length <N>`      | 3          | Default array length             |
| `--max-depth <N>`        | 5          | Max nesting depth                |
| `--include-deprecated`   | false      | Include deprecated fields        |
| `--generate-variants`    | false      | Generate multiple variants       |
| `--no-auth`              | -          | Skip authentication              |
| `--proxied`              | -          | Endpoint is a ferrimock proxy      |
| `-H, --header <HEADER>`  | -          | Custom headers (repeatable)      |

## Common Issues

| Issue               | Solution                                                                           |
| ------------------- | ---------------------------------------------------------------------------------- |
| Introspection fails | Check auth with `RUST_LOG=debug`, verify endpoint access                           |
| No mocks generated  | Check operation filters (`--queries`, `--mutations`), verify schema has operations |
| Wrong fake data     | Use `--type-mappings` to override defaults                                         |
| Infinite nesting    | Reduce `--max-depth` (default: 5)                                                  |
| Large file size     | Reduce `--list-length`, `--max-depth`, or exclude operations                       |

## Tips

1. Start with defaults, customize after generation
2. Use `--proxied` when proxy is already running
3. Combine with HAR recordings for complete coverage
4. Higher priority for custom mocks (200+) vs generated (100)
5. Test mocks with `ferrimock mock test`
6. Use GraphQL REPL to verify schema before generating

## See Also

- [Mock Server Guide](./MOCK_SERVER.md) - Mock system features
- [GraphQL REPL](./GRAPHQL_REPL.md) - Interactive GraphQL testing
- [Proxy Guide](./PROXY.md) - Proxy configuration
- [CLI Reference](./CLI_REFERENCE.md) - Complete command reference
