//! Fake data generation functions

use crate::ui;

use super::generators::{GeneratorInfo, get_all_generators};

/// Generate fake data values
#[allow(clippy::too_many_arguments)]
pub fn generate_fake_data(
    generator: &str,
    count: usize,
    min: Option<f64>,
    max: Option<f64>,
    words: Option<usize>,
    length: Option<usize>,
    format: &str,
    copy: bool,
) -> anyhow::Result<()> {
    let mut results: Vec<String> = Vec::with_capacity(count);

    for _ in 0..count {
        let value = generate_single_value(generator, min, max, words, length)?;
        results.push(value);
    }

    // Format output
    let output = match format {
        "json" => {
            // Parse results that are already valid JSON (e.g. composite generators
            // like `user` and `address` return JSON objects/arrays, and `number`/
            // `boolean` return parseable literals). Fall back to string for plain text.
            let json_values: Vec<serde_json::Value> = results
                .iter()
                .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::String(s.clone())))
                .collect();
            serde_json::to_string_pretty(&json_values)?
        }
        "csv" => results.join(","),
        _ => results.join("\n"),
    };

    // Copy to clipboard if requested (feature not currently enabled)
    if copy {
        eprintln!(
            "{}",
            ui::warning("Clipboard support not enabled. Output is printed below.")
        );
    }

    println!("{output}");
    Ok(())
}

/// Generate a single fake value based on generator type
pub fn generate_single_value(
    generator: &str,
    min: Option<f64>,
    max: Option<f64>,
    words: Option<usize>,
    length: Option<usize>,
) -> anyhow::Result<String> {
    use mockpit_fake_data::*;

    let value = match generator.to_lowercase().as_str() {
        // Identity
        "name" => fake_name(),
        "first_name" | "firstname" => fake_first_name(),
        "last_name" | "lastname" => fake_last_name(),
        "username" => fake_username(),
        "password" => fake_password(),
        "title" => fake_title(),
        "suffix" => fake_suffix(),

        // Contact
        "email" => fake_email(),
        "free_email" | "freeemail" => fake_free_email(),
        "phone" => fake_phone(),
        "cell_phone" | "cellphone" | "mobile" => fake_cell_phone(),

        // Company
        "company" => fake_company(),
        "company_suffix" | "companysuffix" => fake_company_suffix(),
        "job_title" | "jobtitle" | "job" => fake_job_title(),
        "industry" => fake_industry(),
        "job_field" | "jobfield" => fake_job_field(),
        "job_position" | "jobposition" | "position" => fake_job_position(),
        "job_seniority" | "jobseniority" | "seniority" => fake_job_seniority(),

        // Internet
        "url" => fake_url(),
        "domain" => fake_domain(),
        "ipv4" | "ip" => fake_ipv4(),
        "ipv6" => fake_ipv6(),
        "mac_address" | "macaddress" | "mac" => fake_mac_address(),
        "user_agent" | "useragent" | "ua" => fake_user_agent(),
        "color" => fake_color(),

        // Finance
        "credit_card" | "creditcard" | "cc" => fake_credit_card(),
        "currency_code" | "currencycode" => fake_currency_code(),
        "currency_name" | "currencyname" => fake_currency_name(),
        "currency_symbol" | "currencysymbol" => fake_currency_symbol(),
        "price" => {
            let min_val = min.unwrap_or(1.0);
            let max_val = max.unwrap_or(999.99);
            format!("{:.2}", fake_price(min_val, max_val))
        }
        "amount" => fake_amount(),

        // DateTime
        "date" | "datetime" => fake_date(),
        "time" => fake_time(),
        "iso_date" | "isodate" => fake_iso_date(),
        "unix_timestamp" | "unixtimestamp" | "timestamp" => fake_unix_timestamp().to_string(),
        "relative_time" | "relativetime" => fake_relative_time(),

        // Location
        "street" => fake_street(),
        "street_address" | "streetaddress" => fake_street_address(),
        "city" => fake_city(),
        "state" => fake_state(),
        "state_abbr" | "stateabbr" => fake_state_abbr(),
        "zip" | "zipcode" | "postal_code" | "postalcode" => fake_zip(),
        "country" => fake_country(),
        "country_code" | "countrycode" => fake_country_code(),
        "latitude" | "lat" => fake_latitude(),
        "longitude" | "lng" | "lon" => fake_longitude(),
        "building_number" | "buildingnumber" => fake_building_number(),

        // Text
        "word" => fake_word(),
        "words" => fake_words(words.unwrap_or(5)),
        "sentence" => fake_sentence(words.unwrap_or(10)),
        "paragraph" => fake_paragraph(words.unwrap_or(5)),
        "slug" => fake_slug(),
        "alphanumeric" | "alphanum" => fake_alphanumeric(length.unwrap_or(10)),

        // Identifiers
        "uuid" | "guid" => fake_uuid(),
        "token" => fake_token(),
        "numeric_id" | "numericid" | "id" => fake_numeric_id(),
        "short_hash" | "shorthash" | "hash" => fake_short_hash(),
        "sha256" => fake_sha256(),
        "md5" => fake_md5(),
        "base64" => fake_base64(),
        "jwt" => fake_jwt(),
        "isbn" => fake_isbn(),
        "isbn13" => fake_isbn13(),
        "etag" => fake_etag(),

        // Web
        "boolean" | "bool" => fake_boolean().to_string(),
        "filename" => fake_filename(),
        "file_size" | "filesize" => {
            // Truncation is intentional: user provides f64, API expects i64
            #[allow(clippy::cast_possible_truncation)]
            let min_val = min.map_or(1024, |v| v as i64);
            #[allow(clippy::cast_possible_truncation)]
            let max_val = max.map_or(1_048_576, |v| v as i64);
            fake_file_size(min_val, max_val).to_string()
        }
        "mime_type" | "mimetype" | "mime" => fake_mime_type(),
        "file_extension" | "fileextension" | "ext" => fake_file_extension(),
        "version" | "semver" => fake_version(),
        "hex_color" | "hexcolor" => fake_hex_color(),
        "rgb_color" | "rgbcolor" => fake_rgb_color(),
        "locale" => fake_locale(),
        "timezone" | "tz" => fake_timezone(),
        "number" | "int" | "integer" => {
            // Truncation is intentional: user provides f64, API expects i64
            #[allow(clippy::cast_possible_truncation)]
            let min_val = min.map_or(0, |v| v as i64);
            #[allow(clippy::cast_possible_truncation)]
            let max_val = max.map_or(1000, |v| v as i64);
            fake_number(min_val, max_val).to_string()
        }
        "float" | "double" => {
            let min_val = min.unwrap_or(0.0);
            let max_val = max.unwrap_or(1000.0);
            format!("{:.4}", fake_float(min_val, max_val))
        }
        "digit" => fake_digit().to_string(),

        // Composite types
        "user" => {
            let user = serde_json::json!({
              "id": fake_uuid(),
              "name": fake_name(),
              "email": fake_email(),
              "username": fake_username(),
              "created_at": fake_iso_date()
            });
            serde_json::to_string_pretty(&user)?
        }
        "address" => {
            let address = serde_json::json!({
              "street": fake_street_address(),
              "city": fake_city(),
              "state": fake_state(),
              "zip": fake_zip(),
              "country": fake_country()
            });
            serde_json::to_string_pretty(&address)?
        }

        _ => {
            anyhow::bail!(
                "Unknown generator: '{generator}'. Use 'fake list' to see available generators."
            );
        }
    };

    Ok(value)
}

/// List generators for a specific category
pub fn list_generators_for_category(category: Option<&str>, format: &str) -> anyhow::Result<()> {
    list_generators(category, None, true, format)
}

/// List all available generators with optional filtering
pub fn list_generators(
    category: Option<&str>,
    search: Option<&str>,
    verbose: bool,
    format: &str,
) -> anyhow::Result<()> {
    let all_generators = get_all_generators();

    // Filter by category and/or search
    let filtered: Vec<_> = all_generators
        .iter()
        .filter(|g| {
            let category_match = category.is_none_or(|c| g.category.eq_ignore_ascii_case(c));
            let search_match = search.is_none_or(|s| {
                let s = s.to_lowercase();
                g.name.to_lowercase().contains(&s)
                    || g.description.to_lowercase().contains(&s)
                    || g.category.to_lowercase().contains(&s)
            });
            category_match && search_match
        })
        .collect();

    if format == "json" {
        let json: Vec<_> = filtered
            .iter()
            .map(|g| {
                serde_json::json!({
                  "name": g.name,
                  "category": g.category,
                  "description": g.description,
                  "example": g.example,
                  "params": g.params
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    println!("{}", ui::header("Available Fake Data Generators"));
    println!();

    // Group by category
    let mut categories: Vec<&str> = filtered.iter().map(|g| g.category).collect();
    categories.sort_unstable();
    categories.dedup();

    for cat in categories {
        println!("{}", ui::emphasis(&cat.to_uppercase()));
        println!();

        let cat_generators: Vec<&&GeneratorInfo> =
            filtered.iter().filter(|g| g.category == cat).collect();

        for g in cat_generators {
            if verbose {
                println!(
                    "  {} {} {}",
                    ui::action(g.name),
                    g.description,
                    ui::dim(g.params)
                );
                println!("    {} {}", ui::dim("Example:"), g.example);
                println!();
            } else {
                println!("  {} {}", ui::action(g.name), g.description);
            }
        }

        if !verbose {
            println!();
        }
    }

    Ok(())
}
