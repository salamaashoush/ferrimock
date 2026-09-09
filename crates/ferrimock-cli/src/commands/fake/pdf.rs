//! Fake PDF generation

use crate::commands::ui;
use crate::ops::fake::Pdf;
use base64::Engine as _;
use ferrimock::fake_data::document::{Extras, PdfSpec, fake_pdf_document};
use ferrimock::fake_data::pdf::Meta;

/// Parse a `AxB` pair, as `--table 6x3` and `--image 960x280` take.
fn parse_pair(value: &str, flag: &str) -> anyhow::Result<(u32, u32)> {
    let (left, right) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("--{flag} wants AxB, got {value:?}"))?;
    let parse = |part: &str| {
        part.trim()
            .parse::<u32>()
            .map_err(|_| anyhow::anyhow!("--{flag} wants numbers, got {value:?}"))
    };
    Ok((parse(left)?, parse(right)?))
}

/// Parse `KIND:POINTS`, as `--chart bar:8` takes. The count is optional.
fn parse_chart(value: &str) -> anyhow::Result<(ferrimock::fake_data::pdf::ChartKind, usize)> {
    let (kind, points) = value.split_once(':').unwrap_or((value, "8"));
    let kind = kind
        .parse()
        .map_err(|e: String| anyhow::anyhow!("--chart: {e}"))?;
    let points = points
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("--chart wants KIND:POINTS, got {value:?}"))?;
    Ok((kind, points))
}

/// Parse an optional flag, naming the flag in the error the way clap would.
fn parse_opt<T>(value: Option<&str>, flag: &str) -> anyhow::Result<Option<T>>
where
    T: std::str::FromStr<Err = String>,
{
    value
        .map(str::parse)
        .transpose()
        .map_err(|e: String| anyhow::anyhow!("--{flag}: {e}"))
}

pub fn build_spec(opts: &Pdf) -> anyhow::Result<PdfSpec> {
    let mut extras = Extras {
        paragraphs: opts.paragraphs,
        callouts: opts.callouts,
        lists: opts.lists.clone(),
        key_values: opts.key_values.clone(),
        ..Extras::default()
    };
    for table in &opts.tables {
        let (rows, columns) = parse_pair(table, "table")?;
        extras.tables.push((rows as usize, columns as usize));
    }
    for image in &opts.images {
        extras.images.push(parse_pair(image, "image")?);
    }
    for chart in &opts.charts {
        extras.charts.push(parse_chart(chart)?);
    }
    if let Some(columns) = &opts.columns {
        let (count, paragraphs) = parse_pair(columns, "columns")?;
        extras.columns = Some((count as usize, paragraphs as usize));
    }

    let accent = match &opts.accent {
        Some(hex) => Some(
            ferrimock::fake_data::pdf::Rgb::parse(hex)
                .ok_or_else(|| anyhow::anyhow!("--accent wants a hex colour, got {hex:?}"))?,
        ),
        None => None,
    };

    Ok(PdfSpec {
        pages: opts.pages,
        text: opts.text.clone(),
        title: opts.title.clone(),
        preset: opts
            .preset
            .parse()
            .map_err(|e: String| anyhow::anyhow!(e))?,
        repeat: opts.repeat,
        extras,
        page_size: opts
            .page_size
            .parse()
            .map_err(|e: String| anyhow::anyhow!("--page-size: {e}"))?,
        orientation: parse_opt(opts.orientation.as_deref(), "orientation")?,
        family: parse_opt(opts.font.as_deref(), "font")?,
        accent,
        margin: opts.margin,
        watermark: opts.watermark.clone(),
        header: opts.header.clone(),
        footer: opts.footer.clone(),
        page_numbers: opts.no_page_numbers.then_some(false),
        meta: Meta {
            title: opts.title.clone(),
            author: opts.author.clone(),
            subject: opts.subject.clone(),
            keywords: opts.keywords.clone(),
            creator: None,
        },
    })
}

/// Where the nth document of a batch goes. `{n}` in the path is the index, so a
/// batch does not write every document over the same file.
fn output_path(template: &str, index: usize, count: usize) -> String {
    if template.contains("{n}") {
        return template.replace(
            "{n}",
            &format!("{:0width$}", index + 1, width = width(count)),
        );
    }
    if count == 1 {
        return template.to_string();
    }
    match template.rsplit_once('.') {
        Some((stem, extension)) => format!(
            "{stem}-{:0width$}.{extension}",
            index + 1,
            width = width(count)
        ),
        None => format!("{template}-{:0width$}", index + 1, width = width(count)),
    }
}

/// Digits needed to number a batch, so the paths sort in order.
fn width(count: usize) -> usize {
    count.to_string().len()
}

pub fn generate_fake_pdf(opts: &Pdf) -> anyhow::Result<()> {
    let spec = build_spec(opts)?;
    let count = opts.count.max(1);

    if count > 1 && opts.output.is_none() && !opts.base64 && !opts.data_uri {
        anyhow::bail!(
            "--count {count} needs --output, or every document goes to its own temp file"
        );
    }

    for index in 0..count {
        let composed = fake_pdf_document(&spec);

        // Content that does not fit is dropped rather than drawn off the page,
        // so say so: a fixture that silently loses most of its text is worse
        // than one that fails.
        if let Some(first) = composed.overflowed_pages.first() {
            let pages = composed.overflowed_pages.len();
            crate::say!(
                "{}",
                ui::warning(&format!(
                    "A block was too tall for the page on {pages} page(s), first is page {first}. \
                     Shorten it, or use a larger --page-size."
                ))
            );
        }

        let base64_data = composed.base64;

        if let Some(path) = &opts.output {
            let path = output_path(path, index, count);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&base64_data)
                .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
            if let Some(parent) = std::path::Path::new(&path).parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &bytes)?;
            crate::say!(
                "{}",
                ui::success(&format!(
                    "Saved {} page(s) to {}",
                    composed.pages,
                    ui::path(&path)
                ))
            );

            if opts.open {
                let _ = open::that(&path);
            }
        } else if opts.data_uri {
            println!("data:application/pdf;base64,{base64_data}");
        } else if opts.base64 {
            println!("{base64_data}");
        } else {
            let temp_path = std::env::temp_dir().join(format!(
                "fake-document-{}.pdf",
                ferrimock::fake_data::fake_short_hash()
            ));
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&base64_data)
                .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
            std::fs::write(&temp_path, &bytes)?;
            println!(
                "{}",
                ui::success(&format!(
                    "Generated {} page(s): {}",
                    composed.pages,
                    ui::path(&temp_path.to_string_lossy())
                ))
            );

            if opts.open {
                let _ = open::that(&temp_path);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pairs_parse_both_separators() {
        assert_eq!(parse_pair("6x3", "table").unwrap(), (6, 3));
        assert_eq!(parse_pair("960X280", "image").unwrap(), (960, 280));
    }

    #[test]
    fn a_pair_without_a_separator_is_rejected() {
        assert!(parse_pair("6", "table").is_err());
        assert!(parse_pair("axb", "table").is_err());
    }

    #[test]
    fn a_chart_takes_a_default_point_count() {
        assert_eq!(
            parse_chart("bar").unwrap().0,
            ferrimock::fake_data::pdf::ChartKind::Bar
        );
        assert_eq!(parse_chart("line:12").unwrap().1, 12);
        assert!(parse_chart("spiral:4").is_err());
    }

    #[test]
    fn a_batch_numbers_its_own_paths() {
        assert_eq!(output_path("out/doc.pdf", 0, 1), "out/doc.pdf");
        assert_eq!(output_path("out/doc.pdf", 4, 12), "out/doc-05.pdf");
        assert_eq!(output_path("out/{n}-doc.pdf", 0, 9), "out/1-doc.pdf");
        assert_eq!(output_path("doc", 2, 3), "doc-3");
    }

    #[test]
    fn every_preset_name_builds_a_spec() {
        for preset in ferrimock::fake_data::document::PdfPreset::ALL {
            let opts = Pdf {
                preset: preset.name().to_string(),
                ..Pdf::default()
            };
            assert!(build_spec(&opts).is_ok(), "{}", preset.name());
        }
    }

    #[test]
    fn a_bad_flag_names_itself() {
        let cases = [
            Pdf {
                page_size: "a9".into(),
                ..Pdf::default()
            },
            Pdf {
                orientation: Some("sideways".into()),
                ..Pdf::default()
            },
            Pdf {
                font: Some("comic".into()),
                ..Pdf::default()
            },
            Pdf {
                accent: Some("blurple".into()),
                ..Pdf::default()
            },
        ];
        for opts in cases {
            assert!(build_spec(&opts).is_err());
        }
    }
}
