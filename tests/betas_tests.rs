use warp_byok_proxy::betas::build_betas;

#[test]
fn opus_1m_enabled_injects_context_1m_beta() {
    let b = build_betas(true, &[]);
    assert_eq!(b, vec!["context-1m-2025-08-07"]);
}

#[test]
fn no_1m_means_no_beta() {
    let b = build_betas(false, &[]);
    assert!(b.is_empty());
}

#[test]
fn preserves_existing_betas_and_dedupes() {
    let b = build_betas(true, &["prompt-caching-2024-07-31"]);
    assert_eq!(b.len(), 2);
    assert!(b.contains(&"context-1m-2025-08-07".to_string()));
    assert!(b.contains(&"prompt-caching-2024-07-31".to_string()));
}

#[test]
fn no_duplicate_when_caller_already_passed_context_1m() {
    let b = build_betas(true, &["context-1m-2025-08-07"]);
    assert_eq!(b, vec!["context-1m-2025-08-07"]);
}
