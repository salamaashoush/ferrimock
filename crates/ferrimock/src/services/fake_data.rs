//! Fake data generation service.

/// Input for generating fake data.
#[derive(Debug, Clone, Default)]
pub struct FakeDataInput {
    /// Generator name (e.g., "email", "name", "uuid")
    pub generator: String,
    /// Number of values to generate
    pub count: usize,
    /// Minimum value (for numeric generators)
    pub min: Option<f64>,
    /// Maximum value (for numeric generators)
    pub max: Option<f64>,
    /// Word count (for text generators)
    pub words: Option<usize>,
    /// Length (for alphanumeric/token generators)
    pub length: Option<usize>,
}

/// Information about a fake data generator.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GeneratorInfo {
    /// Generator name
    pub name: String,
    /// Category (Identity, Contact, Internet, etc.)
    pub category: String,
    /// Short description
    pub description: String,
    /// Example output
    pub example: String,
}

/// Generate fake data values.
#[allow(clippy::needless_pass_by_value)] // owned input is the service API boundary
pub fn generate(input: FakeDataInput) -> Result<Vec<String>, crate::FerrimockError> {
    let mut values = Vec::with_capacity(input.count.max(1));

    for _ in 0..input.count.max(1) {
        let value = generate_single(
            &input.generator,
            input.min,
            input.max,
            input.words,
            input.length,
        )?;
        values.push(value);
    }

    Ok(values)
}

/// Generate a single fake data value.
#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
pub fn generate_single(
    generator: &str,
    min: Option<f64>,
    max: Option<f64>,
    words: Option<usize>,
    length: Option<usize>,
) -> Result<String, crate::FerrimockError> {
    use crate::fake_data::*;

    let result = match normalize_generator(generator) {
        // Identity
        "name" | "full_name" => fake_name(),
        "first_name" => fake_first_name(),
        "last_name" => fake_last_name(),
        "username" => fake_username(),
        "password" => fake_password(),
        "title" => fake_title(),
        "suffix" => fake_suffix(),

        // Contact
        "email" => fake_email(),
        "free_email" => fake_free_email(),
        "phone" | "phone_number" => fake_phone(),
        "cell_phone" => fake_cell_phone(),

        // Company
        "company" | "company_name" => fake_company(),
        "job_title" => fake_job_title(),
        "industry" => fake_industry(),
        "job_field" => fake_job_field(),
        "job_position" => fake_job_position(),
        "job_seniority" => fake_job_seniority(),

        // Internet
        "url" => fake_url(),
        "domain" | "domain_name" => fake_domain(),
        "ipv4" | "ip" => fake_ipv4(),
        "ipv6" => fake_ipv6(),
        "mac_address" | "mac" => fake_mac_address(),
        "user_agent" => fake_user_agent(),
        "color" | "hex_color" => fake_hex_color(),
        "rgb_color" => fake_rgb_color(),

        // Finance
        "credit_card" => fake_credit_card(),
        "currency_code" => fake_currency_code(),
        "currency_name" => fake_currency_name(),
        "currency_symbol" => fake_currency_symbol(),
        "amount" => fake_amount(),

        // DateTime
        "date" | "datetime" | "date_time" => fake_date(),
        "time" => fake_time(),
        "iso_date" => fake_iso_date(),
        "unix_timestamp" => fake_unix_timestamp().to_string(),
        "relative_time" => fake_relative_time(),

        // Identifiers
        "uuid" | "uuid_v4" => fake_uuid(),
        "token" => fake_token(),
        "etag" => fake_etag(),
        "numeric_id" => fake_numeric_id(),
        "short_hash" => fake_short_hash(),
        "sha256" => fake_sha256(),
        "md5" => fake_md5(),
        "base64" => fake_base64(),
        "jwt" => fake_jwt(),
        "isbn" => fake_isbn(),
        "isbn13" => fake_isbn13(),

        // Text
        "word" => fake_word(),
        "words" => fake_words(words.unwrap_or(3)),
        "sentence" => fake_sentence(words.unwrap_or(6)),
        "paragraph" => fake_paragraph(words.unwrap_or(3)),
        "slug" => fake_slug(),
        "alphanumeric" => fake_alphanumeric(length.unwrap_or(32)),

        // Location
        "city" => fake_city(),
        "country" => fake_country(),
        "country_code" => fake_country_code(),
        "timezone" => fake_timezone(),
        "latitude" | "lat" => fake_latitude(),
        "longitude" | "lng" | "lon" => fake_longitude(),

        // Web
        "boolean" | "bool" => fake_boolean().to_string(),
        "filename" => fake_filename(),
        "mime_type" => fake_mime_type(),
        "file_extension" => fake_file_extension(),
        "version" => fake_version(),
        "locale" => fake_locale(),
        "digit" => fake_digit().to_string(),

        // Numbers
        "number" | "integer" | "int" => {
            let min_val = min.map_or(0, |v| v as i64);
            let max_val = max.map_or(1000, |v| v as i64);
            fake_number(min_val, max_val).to_string()
        }
        "float" | "decimal" => {
            let min_val = min.unwrap_or(0.0);
            let max_val = max.unwrap_or(1000.0);
            fake_float(min_val, max_val).to_string()
        }

        // Location (extended)
        "street" => fake_street(),
        "street_address" => fake_street_address(),
        "state" => fake_state(),
        "state_abbr" => fake_state_abbr(),
        "zip" => fake_zip(),
        "building_number" => fake_building_number(),

        // Finance / web (extended)
        "price" => format!(
            "{:.2}",
            fake_price(min.unwrap_or(1.0), max.unwrap_or(999.99))
        ),
        "file_size" => {
            let min_val = min.map_or(1024, |v| v as i64);
            let max_val = max.map_or(1_048_576, |v| v as i64);
            fake_file_size(min_val, max_val).to_string()
        }

        // Composite objects
        "user" => serde_json::to_string_pretty(&serde_json::json!({
            "id": fake_uuid(),
            "name": fake_name(),
            "email": fake_email(),
            "username": fake_username(),
            "created_at": fake_iso_date(),
        }))?,
        "address" => serde_json::to_string_pretty(&serde_json::json!({
            "street": fake_street_address(),
            "city": fake_city(),
            "state": fake_state(),
            "zip": fake_zip(),
            "country": fake_country(),
        }))?,

        other => crate::mp_bail!("Unknown generator: {other}"),
    };

    Ok(result)
}

/// Normalize alternate generator spellings (no-underscore, abbreviations) to the
/// canonical name handled by [`generate_single`]. Keeps a single generator table.
fn normalize_generator(generator: &str) -> &str {
    match generator {
        "firstname" => "first_name",
        "lastname" => "last_name",
        "freeemail" => "free_email",
        "cellphone" | "mobile" => "cell_phone",
        "companysuffix" => "company_suffix",
        "jobtitle" | "job" => "job_title",
        "jobfield" => "job_field",
        "jobposition" | "position" => "job_position",
        "jobseniority" | "seniority" => "job_seniority",
        "macaddress" => "mac_address",
        "useragent" | "ua" => "user_agent",
        "creditcard" | "cc" => "credit_card",
        "currencycode" => "currency_code",
        "currencyname" => "currency_name",
        "currencysymbol" => "currency_symbol",
        "isodate" => "iso_date",
        "unixtimestamp" | "timestamp" => "unix_timestamp",
        "relativetime" => "relative_time",
        "streetaddress" => "street_address",
        "stateabbr" => "state_abbr",
        "zipcode" | "postal_code" | "postalcode" => "zip",
        "countrycode" => "country_code",
        "buildingnumber" => "building_number",
        "alphanum" => "alphanumeric",
        "guid" => "uuid",
        "id" | "numericid" => "numeric_id",
        "shorthash" | "hash" => "short_hash",
        "filesize" => "file_size",
        "mimetype" | "mime" => "mime_type",
        "fileextension" | "ext" => "file_extension",
        "semver" => "version",
        "hexcolor" => "hex_color",
        "rgbcolor" => "rgb_color",
        "tz" => "timezone",
        "double" => "float",
        "datetime" => "date",
        other => other,
    }
}

/// List all available generators.
pub fn list_generators(category: Option<&str>, search: Option<&str>) -> Vec<GeneratorInfo> {
    let all = all_generators();

    all.into_iter()
        .filter(|g| category.is_none_or(|c| g.category.eq_ignore_ascii_case(c)))
        .filter(|g| {
            search.is_none_or(|s| {
                g.name.contains(s) || g.description.to_lowercase().contains(&s.to_lowercase())
            })
        })
        .collect()
}

fn all_generators() -> Vec<GeneratorInfo> {
    vec![
        generator_info("name", "Identity", "Full name", "John Doe"),
        generator_info("first_name", "Identity", "First name", "John"),
        generator_info("last_name", "Identity", "Last name", "Doe"),
        generator_info("username", "Identity", "Username", "john_doe42"),
        generator_info("password", "Identity", "Password", "xK9#mP2$vL"),
        generator_info("email", "Contact", "Email address", "john@example.com"),
        generator_info("free_email", "Contact", "Free email", "john@gmail.com"),
        generator_info("phone", "Contact", "Phone number", "+1-555-0123"),
        generator_info("company", "Company", "Company name", "Acme Corp"),
        generator_info("job_title", "Company", "Job title", "Software Engineer"),
        generator_info("url", "Internet", "URL", "https://example.com"),
        generator_info("domain", "Internet", "Domain name", "example.com"),
        generator_info("ipv4", "Internet", "IPv4 address", "192.168.1.1"),
        generator_info("ipv6", "Internet", "IPv6 address", "2001:db8::1"),
        generator_info(
            "uuid",
            "Identifiers",
            "UUID v4",
            "550e8400-e29b-41d4-a716-446655440000",
        ),
        generator_info("token", "Identifiers", "Auth token", "a1b2c3d4e5f6"),
        generator_info("date", "DateTime", "Date (RFC3339)", "2024-01-15T10:30:00Z"),
        generator_info("time", "DateTime", "Time", "14:30:00"),
        generator_info("iso_date", "DateTime", "ISO date", "2024-01-15"),
        generator_info("word", "Text", "Single word", "lorem"),
        generator_info(
            "sentence",
            "Text",
            "Sentence",
            "Lorem ipsum dolor sit amet.",
        ),
        generator_info("slug", "Text", "URL slug", "lorem-ipsum-dolor"),
        generator_info("city", "Location", "City name", "New York"),
        generator_info("country", "Location", "Country name", "United States"),
        generator_info("boolean", "Web", "Boolean", "true"),
        generator_info("number", "Web", "Integer (min/max)", "42"),
        generator_info("float", "Web", "Float (min/max)", "3.14"),
        generator_info(
            "credit_card",
            "Finance",
            "Credit card number",
            "4111111111111111",
        ),
        generator_info("currency_code", "Finance", "Currency code", "USD"),
        generator_info("hex_color", "Internet", "Hex color", "#FF5733"),
        generator_info(
            "user_agent",
            "Internet",
            "User agent string",
            "Mozilla/5.0 ...",
        ),
    ]
}

fn generator_info(name: &str, category: &str, description: &str, example: &str) -> GeneratorInfo {
    GeneratorInfo {
        name: name.into(),
        category: category.into(),
        description: description.into(),
        example: example.into(),
    }
}
