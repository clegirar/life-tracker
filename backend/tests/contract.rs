//! Les fixtures du contrat contre le schéma JSON (sans base).

mod common;
use common::*;
use serde_json::json;

#[test]
fn valid_fixtures_match_the_schema() {
    let schema = validator(None);
    for (name, fixture) in fixtures("valid") {
        let errors: Vec<_> = schema
            .iter_errors(&fixture)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{name} : {errors:?}");
    }
}

#[test]
fn invalid_fixtures_are_refused_by_the_schema() {
    let schema = validator(None);
    for (name, fixture) in fixtures("invalid") {
        assert!(!schema.is_valid(&fixture), "{name} devrait être refusé");
    }
}

#[test]
fn schema_caps_the_batch_at_500() {
    let schema = validator(None);
    let batch = |n: usize| json!({ "records": vec![record("k"); n] });
    assert!(schema.is_valid(&batch(500)));
    assert!(!schema.is_valid(&batch(501)));
}
