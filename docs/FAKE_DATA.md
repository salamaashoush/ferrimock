---
sidebar_label: Fake Data
section: Tools
order: 6
---

# Fake Data Generator

Generate fake data, images, PDFs, and more directly from the CLI. Useful for testing, prototyping, and mock data
generation.

## Quick Start

```bash
# Generate random data
mockpit fake data name
mockpit fake data email -n 5
mockpit fake data uuid -f json

# Generate images
mockpit fake image placeholder -W 800 -H 600
mockpit fake image avatar -i "JS" -s 128

# Generate PDFs
mockpit fake pdf -p 3 -o report.pdf

# Preview templates
mockpit fake preview '{"user": "{{ fake_name() }}", "id": "{{ fake_uuid() }}"}'

# Start HTTP server
mockpit fake serve -p 8080
```

## Data Generation

Generate fake data for testing and development.

```bash
mockpit fake data <GENERATOR> [OPTIONS]
```

### Basic Usage

```bash
# Single value
mockpit fake data name
# Output: John Smith

# Multiple values
mockpit fake data email -n 5

# JSON output
mockpit fake data uuid -f json
# Output: {"uuid": "550e8400-e29b-41d4-a716-446655440000"}

# Copy to clipboard
mockpit fake data password -c
```

### Generator Options

| Option         | Description       | Example         |
| -------------- | ----------------- | --------------- |
| `-n, --count`  | Number of values  | `--count 10`    |
| `--min`        | Minimum value     | `--min 1`       |
| `--max`        | Maximum value     | `--max 100`     |
| `-w, --words`  | Word count        | `--words 5`     |
| `-l, --length` | String length     | `--length 32`   |
| `-f, --format` | Output format     | `--format json` |
| `-c, --copy`   | Copy to clipboard | Flag            |

### Output Formats

```bash
# Plain text (default)
mockpit fake data name
# John Smith

# JSON
mockpit fake data name -f json
# {"name": "John Smith"}

# CSV (for multiple values)
mockpit fake data name -n 3 -f csv
# name
# John Smith
# Jane Doe
# Bob Johnson
```

## Available Generators

### Identity

| Generator    | Description       | Example      |
| ------------ | ----------------- | ------------ |
| `name`       | Full name         | John Smith   |
| `first_name` | First name        | John         |
| `last_name`  | Last name         | Smith        |
| `username`   | Username          | john_smith42 |
| `password`   | Random password   | xK9#mP2$vL   |
| `title`      | Title (Mr., Mrs.) | Mr.          |
| `suffix`     | Name suffix       | Jr.          |

### Contact

| Generator    | Description         | Example                |
| ------------ | ------------------- | ---------------------- |
| `email`      | Email address       | john.smith@example.com |
| `free_email` | Free email provider | john.smith@gmail.com   |
| `phone`      | Phone number        | (555) 123-4567         |
| `cell_phone` | Cell phone          | (555) 987-6543         |

### Company

| Generator        | Description    | Example             |
| ---------------- | -------------- | ------------------- |
| `company`        | Company name   | TechCorp Industries |
| `company_suffix` | Company suffix | Inc.                |
| `job_title`      | Job title      | Software Engineer   |
| `industry`       | Industry name  | Technology          |
| `job_field`      | Job field      | Engineering         |
| `job_position`   | Position level | Manager             |
| `job_seniority`  | Seniority      | Senior              |

### Internet

| Generator     | Description        | Example                        |
| ------------- | ------------------ | ------------------------------ |
| `url`         | Full URL           | https://example.com/page       |
| `domain`      | Domain name        | example.com                    |
| `ipv4`        | IPv4 address       | 192.168.1.1                    |
| `ipv6`        | IPv6 address       | 2001:0db8:85a3::8a2e:0370:7334 |
| `mac_address` | MAC address        | 00:1A:2B:3C:4D:5E              |
| `user_agent`  | Browser user agent | Mozilla/5.0...                 |
| `color`       | Hex color code     | #FF5722                        |

### Finance

| Generator         | Description      | Example          |
| ----------------- | ---------------- | ---------------- |
| `credit_card`     | Credit card      | 4532015112830366 |
| `currency_code`   | Currency code    | USD              |
| `currency_name`   | Currency name    | US Dollar        |
| `currency_symbol` | Currency symbol  | $                |
| `price`           | Random price     | 42.99            |
| `amount`          | Formatted amount | 1,234.56         |

### DateTime

| Generator        | Description      | Example              |
| ---------------- | ---------------- | -------------------- |
| `date`           | RFC3339 datetime | 2024-01-15T10:30:45Z |
| `time`           | Time only        | 10:30:45             |
| `iso_date`       | ISO date         | 2024-01-15           |
| `unix_timestamp` | Unix timestamp   | 1705318245           |
| `relative_time`  | Relative time    | 2 hours ago          |

### Location

| Generator        | Description          | Example       |
| ---------------- | -------------------- | ------------- |
| `street`         | Street name          | Main Street   |
| `street_address` | Full street address  | 123 Main St   |
| `city`           | City name            | New York      |
| `state`          | State name           | California    |
| `state_abbr`     | State abbreviation   | CA            |
| `zip`            | ZIP code             | 90210         |
| `country`        | Country name         | United States |
| `country_code`   | Country code         | US            |
| `latitude`       | Geographic latitude  | 40.7128       |
| `longitude`      | Geographic longitude | -74.0060      |

### Text

| Generator      | Description         | Example              | Params        |
| -------------- | ------------------- | -------------------- | ------------- |
| `word`         | Single word         | lorem                |               |
| `words`        | Multiple words      | lorem ipsum dolor    | `-w/--words`  |
| `sentence`     | Sentence            | Lorem ipsum dolor... | `-w/--words`  |
| `paragraph`    | Paragraph           | Lorem ipsum dolor... | `-w/--words`  |
| `slug`         | URL-friendly slug   | lorem-ipsum-dolor    |               |
| `alphanumeric` | Alphanumeric string | a1b2c3d4e5           | `-l/--length` |

### Identifiers

| Generator    | Description            | Example                              |
| ------------ | ---------------------- | ------------------------------------ |
| `uuid`       | UUID v4                | 550e8400-e29b-41d4-a716-446655440000 |
| `token`      | 32-char token          | a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6     |
| `numeric_id` | Numeric ID             | 1234567890123                        |
| `short_hash` | Short hash (git-style) | a1b2c3d                              |
| `sha256`     | SHA-256 hash           | 3f78c4d0f5e6a7b8...                  |
| `md5`        | MD5 hash               | d41d8cd98f00b204...                  |
| `base64`     | Base64 string          | SGVsbG8gV29ybGQ=                     |
| `jwt`        | JWT-like token         | eyJhbGciOi...                        |
| `isbn`       | ISBN-10                | 0-123-45678-9                        |
| `isbn13`     | ISBN-13                | 978-0-123-45678-6                    |
| `etag`       | HTTP ETag              | "abc123def456"                       |

### Web/Misc

| Generator        | Description       | Example          | Params         |
| ---------------- | ----------------- | ---------------- | -------------- |
| `boolean`        | Random boolean    | true             |                |
| `filename`       | Filename          | document.pdf     |                |
| `file_size`      | File size (bytes) | 1048576          | `--min, --max` |
| `mime_type`      | MIME type         | application/json |                |
| `file_extension` | File extension    | pdf              |                |
| `version`        | Semantic version  | 1.2.3            |                |
| `semver`         | Semantic version  | 1.2.3            |                |
| `hex_color`      | Hex color         | #FF5722          |                |
| `rgb_color`      | RGB color         | rgb(255, 87, 34) |                |
| `locale`         | Locale code       | en-US            |                |
| `timezone`       | Timezone name     | America/New_York |                |
| `number`         | Random integer    | 42               | `--min, --max` |
| `float`          | Random float      | 3.14159          | `--min, --max` |
| `digit`          | Single digit      | 7                |                |

### Composite Types

| Generator | Description                         | Example                         |
| --------- | ----------------------------------- | ------------------------------- |
| `user`    | User object (name, email, id, etc.) | JSON object with user fields    |
| `address` | Address object                      | JSON object with address fields |

## Listing Generators

```bash
# List all generators
mockpit fake list

# List by category
mockpit fake list --category identity
mockpit fake list --category finance

# Search generators
mockpit fake list --search email

# Verbose output with examples
mockpit fake list --verbose

# JSON output
mockpit fake list --format json
```

### Available Categories

- `identity` - Names, usernames, passwords
- `contact` - Emails, phone numbers
- `company` - Companies, job titles
- `internet` - URLs, IPs, domains
- `finance` - Credit cards, currencies
- `datetime` - Dates, times, timestamps
- `location` - Addresses, cities, countries
- `text` - Words, sentences, paragraphs
- `identifiers` - UUIDs, tokens, hashes
- `web` - Filenames, MIME types, colors
- `composite` - User, address objects

## Image Generation

Generate various types of placeholder images.

```bash
mockpit fake image <TYPE> [OPTIONS]
```

### Image Types

| Type           | Description                 | Key Options                  |
| -------------- | --------------------------- | ---------------------------- |
| `placeholder`  | Placeholder with dimensions | `--text`                     |
| `avatar`       | Avatar with initials        | `--initials`                 |
| `gradient`     | Gradient image              | `--start`, `--end`           |
| `checkerboard` | Checkerboard pattern        | `--bg-color`, `--text-color` |
| `noise`        | Random noise                | `--colored`                  |
| `stripes`      | Striped pattern             | `--direction`                |
| `text`         | Image with custom text      | `--text`                     |
| `solid`        | Solid color                 | `--bg-color`                 |

### Examples

```bash
# Placeholder image
mockpit fake image placeholder -W 800 -H 600

# Avatar with initials
mockpit fake image avatar -i "JS" -s 128

# Gradient
mockpit fake image gradient --start "#FF0000" --end "#0000FF" -d horizontal

# Checkerboard
mockpit fake image checkerboard -W 200 -H 200

# Noise (monochrome or colored)
mockpit fake image noise -W 256 -H 256 --colored

# Stripes
mockpit fake image stripes -d vertical -b "#FFFFFF" -t "#000000"

# Text on image
mockpit fake image text --text "Hello World" -W 400 -H 100

# Solid color
mockpit fake image solid -b "#FF5722" -W 100 -H 100
```

### Image Options

| Option               | Description                | Default    |
| -------------------- | -------------------------- | ---------- |
| `-W, --width`        | Image width                | 200        |
| `-H, --height`       | Image height               | 200        |
| `-s, --size`         | Square size (width=height) | -          |
| `-b, --bg-color`     | Background color           | #CCCCCC    |
| `-t, --text-color`   | Text/foreground color      | #333333    |
| `--text`             | Custom text                | Dimensions |
| `-i, --initials`     | Avatar initials            | ??         |
| `--start`            | Gradient start color       | #FF0000    |
| `--end`              | Gradient end color         | #0000FF    |
| `-d, --direction`    | Gradient/stripe direction  | horizontal |
| `-F, --image-format` | Output format (png/jpeg)   | png        |
| `-q, --quality`      | JPEG quality (1-100)       | 85         |
| `-o, --output`       | Output file path           | temp file  |
| `--base64`           | Output as base64           | false      |
| `--data-uri`         | Output as data URI         | false      |
| `--colored`          | Colored noise              | false      |
| `--open`             | Open after generation      | false      |

### Output Options

```bash
# Save to file
mockpit fake image gradient -o gradient.png

# Output as base64
mockpit fake image avatar -i "AB" --base64

# Output as data URI (for HTML/CSS)
mockpit fake image placeholder --data-uri

# Generate JPEG
mockpit fake image noise -F jpeg -q 90

# Open in default viewer
mockpit fake image checkerboard --open
```

## PDF Generation

Generate fake PDF documents.

```bash
mockpit fake pdf [OPTIONS]
```

### Examples

```bash
# Single page PDF
mockpit fake pdf

# Multi-page PDF
mockpit fake pdf -p 5

# With custom text
mockpit fake pdf -t "Invoice #12345"

# Save to file
mockpit fake pdf -p 3 -o report.pdf

# Output as base64
mockpit fake pdf --base64

# Output as data URI
mockpit fake pdf --data-uri

# Open after generation
mockpit fake pdf -p 2 --open
```

### PDF Options

| Option         | Description           | Default    |
| -------------- | --------------------- | ---------- |
| `-p, --pages`  | Number of pages       | 1          |
| `-t, --text`   | Custom text content   | Lorem text |
| `-o, --output` | Output file path      | temp file  |
| `--base64`     | Output as base64      | false      |
| `--data-uri`   | Output as data URI    | false      |
| `--open`       | Open after generation | false      |

## Template Preview

Preview and render templates with fake data using Tera template syntax.

```bash
mockpit fake preview [TEMPLATE] [OPTIONS]
```

### Examples

```bash
# Inline template
mockpit fake preview '{"name": "{{ fake_name() }}", "email": "{{ fake_email() }}"}'

# From file
mockpit fake preview -f template.json

# With custom context
mockpit fake preview '{"greeting": "Hello {{ name }}"}' -c '{"name": "World"}'

# Multiple renders
mockpit fake preview '{"id": "{{ fake_uuid() }}"}' -n 5

# JSON output
mockpit fake preview '{"user": "{{ fake_name() }}"}' -F json

# CSV output (for multiple)
mockpit fake preview '{{ fake_email() }}' -n 10 -F csv
```

### Template Functions

Templates use Tera syntax with access to all fake data generators:

```
{{ fake_name() }}           - Random name
{{ fake_email() }}          - Random email
{{ fake_uuid() }}           - Random UUID
{{ fake_number() }}         - Random number
{{ fake_date() }}           - Random date
{{ fake_company() }}        - Random company
{{ fake_sentence() }}       - Random sentence
{{ fake_paragraph() }}      - Random paragraph
{{ fake_ipv4() }}           - Random IPv4
{{ fake_credit_card() }}    - Random credit card
```

### Preview Options

| Option          | Description        | Default |
| --------------- | ------------------ | ------- |
| `TEMPLATE`      | Template string    | stdin   |
| `-f, --file`    | Template file path | -       |
| `-c, --context` | JSON context vars  | {}      |
| `-n, --count`   | Number of renders  | 1       |
| `-F, --format`  | Output format      | text    |

### Context Variables

Pass custom variables to templates:

```bash
# Pass context as JSON
mockpit fake preview '{"msg": "Hello {{ name }}, your code is {{ code }}"}' \
  -c '{"name": "Alice", "code": "ABC123"}'

# Mix with fake data
mockpit fake preview '{"user": "{{ name }}", "id": "{{ fake_uuid() }}"}' \
  -c '{"name": "Custom Name"}'
```

## HTTP Server

Run a local HTTP server that provides fake data via REST endpoints.

```bash
mockpit fake serve [OPTIONS]
```

### Starting the Server

```bash
# Default (port 3005)
mockpit fake serve

# Custom port
mockpit fake serve --port 8080

# Custom host
mockpit fake serve --host 0.0.0.0 --port 8080

# Enable CORS
mockpit fake serve --cors

# Verbose logging
mockpit fake serve --verbose

# Open browser
mockpit fake serve --open
```

### Server Options

| Option          | Description     | Default   |
| --------------- | --------------- | --------- |
| `-p, --port`    | Server port     | 3005      |
| `--host`        | Server host     | 127.0.0.1 |
| `--cors`        | Enable CORS     | false     |
| `-o, --open`    | Open browser    | false     |
| `-v, --verbose` | Verbose logging | false     |

### API Endpoints

#### GET / - API Documentation

Returns HTML documentation page.

#### GET /fake/:type - Generate Fake Data

Generate fake data by type.

```bash
# Examples
curl http://localhost:3005/fake/name
# {"value": "John Smith"}

curl http://localhost:3005/fake/email
# {"value": "john.smith@example.com"}

curl http://localhost:3005/fake/uuid
# {"value": "550e8400-e29b-41d4-a716-446655440000"}

curl http://localhost:3005/fake/user
# {"id": "...", "name": "...", "email": "...", "username": "...", "created_at": "..."}

# With parameters
curl "http://localhost:3005/fake/number?min=1&max=100"
# {"value": 42}

curl "http://localhost:3005/fake/words?count=5"
# {"value": "lorem ipsum dolor sit amet"}
```

#### GET /fake/image/:type - Generate Images

Generate images with query parameters.

```bash
# Placeholder
curl "http://localhost:3005/fake/image/placeholder?width=400&height=300" > image.png

# Avatar
curl "http://localhost:3005/fake/image/avatar?initials=JS&size=128" > avatar.png

# Gradient
curl "http://localhost:3005/fake/image/gradient?start=%23FF0000&end=%23FFFF00" > gradient.png

# Supported types: placeholder, avatar, gradient, checkerboard, noise, stripes, text, solid
```

#### GET /fake/pdf - Generate PDFs

```bash
# Single page
curl http://localhost:3005/fake/pdf > document.pdf

# Multi-page
curl "http://localhost:3005/fake/pdf?pages=5" > document.pdf

# With custom text
curl "http://localhost:3005/fake/pdf?text=Invoice%20123" > invoice.pdf
```

#### POST /render - Render Templates

Render templates with fake data.

```bash
# Simple template
curl -X POST http://localhost:3005/render \
  -H "Content-Type: application/json" \
  -d '{"template": "{\"name\": \"{{ fake_name() }}\"}"}'
# {"name": "John Smith"}

# With context
curl -X POST http://localhost:3005/render \
  -H "Content-Type: application/json" \
  -d '{
    "template": "{\"greeting\": \"Hello {{ name }}\"}",
    "context": {"name": "Alice"}
  }'
# {"greeting": "Hello Alice"}

# Multiple renders
curl -X POST http://localhost:3005/render \
  -H "Content-Type: application/json" \
  -d '{"template": "{{ fake_email() }}", "count": 3}'
# ["user1@example.com", "user2@example.com", "user3@example.com"]
```

## Use Cases

### Testing API Clients

```bash
# Start fake server
mockpit fake serve --port 8080 &

# Test your client
curl http://localhost:8080/fake/user
```

### Generating Test Data

```bash
# Generate user data for tests
mockpit fake data user -n 100 -f json > users.json

# Generate UUIDs
mockpit fake data uuid -n 50 > ids.txt
```

### Creating Mock Responses

```bash
# Template for API response
mockpit fake preview '{
  "status": "success",
  "data": {
    "users": [
      {"id": "{{ fake_uuid() }}", "name": "{{ fake_name() }}", "email": "{{ fake_email() }}"},
      {"id": "{{ fake_uuid() }}", "name": "{{ fake_name() }}", "email": "{{ fake_email() }}"}
    ]
  }
}' -F json
```

### Placeholder Images for UI

```bash
# Generate placeholder images
mockpit fake image placeholder -W 1200 -H 630 -o og-image.png
mockpit fake image avatar -i "AB" -s 64 -o avatar.png
```

### Test Documents

```bash
# Generate test PDF
mockpit fake pdf -p 10 -o test-document.pdf
```

## See Also

- [Mock Server Guide](./MOCK_SERVER.md) - Using fake data in mocks
- [CLI Reference](./CLI_REFERENCE.md) - Complete command reference
- [Template Guide](./MOCK_SERVER.md#templates) - Template syntax details
