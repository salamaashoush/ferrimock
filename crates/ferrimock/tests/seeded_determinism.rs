#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! End-to-end determinism: a seeded run must render byte-identical responses.

use ferrimock::fake_data::rng;
use ferrimock::template::render_template_with_id;
use ferrimock::types::RequestContext;
use serial_test::serial;

const TEMPLATE: &str = r#"{"id": "{{ fake_uuid() }}", "name": "{{ fake_name() }}", "email": "{{ fake_email() }}", "n": {{ get_random(start=1, end=1000) }}}"#;

fn render(mock_id: &str) -> String {
    render_template_with_id(TEMPLATE, &RequestContext::new(), Some(mock_id)).unwrap()
}

fn render_run(mock_id: &str, times: usize) -> Vec<String> {
    rng::set_global_seed(Some(1234));
    (0..times).map(|_| render(mock_id)).collect()
}

#[test]
#[serial]
fn same_seed_replays_the_same_responses() {
    let first = render_run("get-user", 3);
    let second = render_run("get-user", 3);
    assert_eq!(first, second);
    // Successive calls to one mock still vary — a seed pins the sequence, it
    // does not freeze the value.
    assert_ne!(first[0], first[1]);
    rng::set_global_seed(None);
}

#[test]
#[serial]
fn a_different_seed_gives_different_responses() {
    let first = render_run("get-user", 2);
    rng::set_global_seed(Some(4321));
    let other: Vec<String> = (0..2).map(|_| render("get-user")).collect();
    assert_ne!(first, other);
    rng::set_global_seed(None);
}

#[test]
#[serial]
fn mocks_do_not_disturb_each_others_streams() {
    let baseline = render_run("get-user", 1);

    rng::set_global_seed(Some(1234));
    // Another mock renders first this time; `get-user` must be unaffected.
    let _ = render("list-posts");
    let _ = render("list-posts");
    assert_eq!(baseline[0], render("get-user"));
    rng::set_global_seed(None);
}

#[test]
#[serial]
fn unseeded_runs_stay_random() {
    rng::set_global_seed(None);
    assert!(!rng::is_seeded());
    assert_ne!(render("get-user"), render("get-user"));
}

#[test]
#[serial]
fn reset_streams_replays_without_reseeding() {
    let first = render_run("get-user", 2);
    rng::reset_streams();
    let replay: Vec<String> = (0..2).map(|_| render("get-user")).collect();
    assert_eq!(first, replay);
    rng::set_global_seed(None);
}
