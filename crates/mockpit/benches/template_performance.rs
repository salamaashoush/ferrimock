use criterion::{Criterion, criterion_group, criterion_main};
use mockpit::template::{
    RequestContext, hash_template, render_template, render_template_with_hash, validate_template,
};
use rustc_hash::FxHashMap;
use std::hint::black_box;

// ============================================================================
// Benchmark 1: Cache Hit - Render same template repeatedly (hot path)
// ============================================================================

fn bench_render_cache_hit(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_cache_hit");
    group.significance_level(0.05).sample_size(1000);

    let ctx = RequestContext {
        method: "GET".to_string(),
        uri: "/api/2.0/users/123".to_string(),
        path: "/api/2.0/users/123".to_string(),
        captures: {
            let mut m = FxHashMap::default();
            m.insert("id".to_string(), "123".to_string());
            m
        },
        query: FxHashMap::default(),
        headers: {
            let mut m = FxHashMap::default();
            m.insert("content-type".to_string(), "application/json".to_string());
            m
        },
        body: None,
        body_json: None,
        vars: None,
    };

    // Simple variable interpolation (most common case)
    group.bench_function("simple_variable", |b| {
        b.iter(|| render_template(black_box(r#"{"id": "{{ captures.id }}"}"#), black_box(&ctx)));
    });

    // Static template (no variables)
    group.bench_function("static_no_vars", |b| {
        b.iter(|| render_template(black_box(r#"{"status": "ok"}"#), black_box(&ctx)));
    });

    // Multiple variables
    group.bench_function("multiple_variables", |b| {
        b.iter(|| {
      render_template(
        black_box(r#"{"method": "{{ method }}", "path": "{{ path }}", "id": "{{ captures.id }}"}"#),
        black_box(&ctx),
      )
    });
    });

    group.finish();
}

// ============================================================================
// Benchmark 2: Fake Data Functions
// ============================================================================

fn bench_render_fake_data(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_fake_data");
    group.significance_level(0.05).sample_size(500);

    let ctx = RequestContext::new();

    // Single fake function
    group.bench_function("single_fake_name", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"name": "{{ fake_name() }}"}"#),
                black_box(&ctx),
            )
        });
    });

    // UUID generation
    group.bench_function("uuid_generation", |b| {
        b.iter(|| render_template(black_box(r#"{"id": "{{ uuid() }}"}"#), black_box(&ctx)));
    });

    // Multiple fake functions in one template
    group.bench_function("multiple_fake_functions", |b| {
    b.iter(|| {
      render_template(
        black_box(
          r#"{"id": "{{ fake_uuid() }}", "name": "{{ fake_name() }}", "email": "{{ fake_email() }}", "company": "{{ fake_company() }}"}"#,
        ),
        black_box(&ctx),
      )
    });
  });

    group.finish();
}

// ============================================================================
// Benchmark 3: Template with Control Flow
// ============================================================================

fn bench_render_control_flow(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_control_flow");
    group.significance_level(0.05).sample_size(500);

    let ctx = RequestContext {
        method: "GET".to_string(),
        uri: "/api/2.0/users".to_string(),
        path: "/api/2.0/users".to_string(),
        captures: FxHashMap::default(),
        query: FxHashMap::default(),
        headers: FxHashMap::default(),
        body: None,
        body_json: None,
        vars: None,
    };

    // For loop generating array
    group.bench_function("for_loop_5_items", |b| {
    b.iter(|| {
      render_template(
        black_box(
          r#"{"items": [{% for i in range(end=5) %}{"id": {{ i }}, "name": "{{ fake_name() }}"}{% if not loop.last %},{% endif %}{% endfor %}]}"#,
        ),
        black_box(&ctx),
      )
    });
  });

    // Conditional logic
    group.bench_function("if_else_condition", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"type": "{% if method == "GET" %}read{% else %}write{% endif %}"}"#),
                black_box(&ctx),
            )
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 4: Validation (used by MockValidator)
// ============================================================================

#[allow(clippy::result_large_err)]
fn bench_validate_template(c: &mut Criterion) {
    let mut group = c.benchmark_group("validate_template");
    group.significance_level(0.05).sample_size(1000);

    group.bench_function("valid_simple", |b| {
        b.iter(|| validate_template(black_box(r#"{"id": "{{ captures.id }}"}"#)));
    });

    group.bench_function("valid_complex", |b| {
    b.iter(|| {
      validate_template(black_box(
        r#"{"id": "{{ fake_uuid() }}", "items": [{% for i in range(end=3) %}{"name": "{{ fake_name() }}"}{% if not loop.last %},{% endif %}{% endfor %}]}"#,
      ))
    });
  });

    group.bench_function("invalid_syntax", |b| {
        b.iter(|| validate_template(black_box(r#"{"id": "{{ bad_syntax"}"#)));
    });

    group.finish();
}

// ============================================================================
// Benchmark 5: Cache Miss (unique templates)
// ============================================================================

fn bench_render_cache_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_cache_miss");
    group.significance_level(0.05).sample_size(200);

    let ctx = RequestContext::new();

    // Each iteration renders a unique template (forces compilation)
    let mut counter = 0u64;
    group.bench_function("unique_template_compile", |b| {
        b.iter(|| {
            counter += 1;
            let template =
                format!(r#"{{"id": "unique_{counter}", "name": "{{{{ fake_name() }}}}"}}"#);
            render_template(black_box(&template), black_box(&ctx))
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 6: New Features - Date Arithmetic & Array Generation
// ============================================================================

fn bench_new_features(c: &mut Criterion) {
    let mut group = c.benchmark_group("new_features");
    group.significance_level(0.05).sample_size(500);

    let ctx = RequestContext::new();

    // Date arithmetic
    group.bench_function("now_plus_days", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"expires": "{{ now_plus(days=30) }}"}"#),
                black_box(&ctx),
            )
        });
    });

    group.bench_function("now_minus_hours", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"created": "{{ now_minus(hours=2) }}"}"#),
                black_box(&ctx),
            )
        });
    });

    group.bench_function("iso_date_offset", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"due_date": "{{ fake_iso_date_offset(days=14) }}"}"#),
                black_box(&ctx),
            )
        });
    });

    // Array generation
    group.bench_function("fake_array_5_names", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"users": {{ fake_array(type="name", count=5) }}}"#),
                black_box(&ctx),
            )
        });
    });

    group.bench_function("fake_array_10_emails", |b| {
        b.iter(|| {
            render_template(
                black_box(r#"{"emails": {{ fake_array(type="email", count=10) }}}"#),
                black_box(&ctx),
            )
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 7: Pre-computed Hash vs Runtime Hash
// ============================================================================

fn bench_precomputed_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("precomputed_hash");
    group.significance_level(0.05).sample_size(1000);

    let ctx = RequestContext {
        method: "GET".to_string(),
        uri: "/api/2.0/users/123".to_string(),
        path: "/api/2.0/users/123".to_string(),
        captures: {
            let mut m = FxHashMap::default();
            m.insert("id".to_string(), "123".to_string());
            m
        },
        query: FxHashMap::default(),
        headers: FxHashMap::default(),
        body: None,
        body_json: None,
        vars: None,
    };

    let template = r#"{"id": "{{ captures.id }}", "name": "{{ fake_name() }}", "email": "{{ fake_email() }}"}"#;
    let hash = hash_template(template);

    group.bench_function("runtime_hash", |b| {
        b.iter(|| render_template(black_box(template), black_box(&ctx)));
    });

    group.bench_function("precomputed_hash", |b| {
        b.iter(|| {
            render_template_with_hash(black_box(template), black_box(hash), black_box(&ctx), None)
        });
    });

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    benches,
    bench_render_cache_hit,
    bench_render_fake_data,
    bench_render_control_flow,
    bench_validate_template,
    bench_render_cache_miss,
    bench_new_features,
    bench_precomputed_hash,
);

criterion_main!(benches);
