use warp_byok_proxy::model_id::{prepare_model_id, PrepareOpts};

#[test]
fn detect_and_strip_1m_suffix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0:1m",
        &PrepareOpts {
            use_cross_region_inference: false,
            use_global_inference: false,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.canonical, "anthropic.claude-opus-4-7-v1:0");
    assert!(out.opus_1m);
}

#[test]
fn no_1m_suffix_means_false() {
    let out = prepare_model_id(
        "anthropic.claude-haiku-4-5-v1:0",
        &PrepareOpts {
            use_cross_region_inference: false,
            use_global_inference: false,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert!(!out.opus_1m);
}

#[test]
fn us_region_adds_us_prefix_when_cri_enabled() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn eu_region_adds_eu_prefix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "eu-west-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "eu.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn apac_region_adds_apac_prefix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "ap-southeast-2",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "apac.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn global_inference_overrides_cri_prefix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: true,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "global.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn already_prefixed_model_is_left_alone() {
    let out = prepare_model_id(
        "us.anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn ca_region_maps_to_us_prefix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "ca-central-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn unknown_region_falls_back_to_us_prefix() {
    let out = prepare_model_id(
        "anthropic.claude-opus-4-7-v1:0",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "sa-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
}

#[test]
fn already_prefixed_with_1m_suffix_keeps_prefix_and_sets_flag() {
    let out = prepare_model_id(
        "us.anthropic.claude-opus-4-7-v1:0:1m",
        &PrepareOpts {
            use_cross_region_inference: true,
            use_global_inference: false,
            region_hint: "us-east-1",
        },
    )
    .unwrap();
    assert_eq!(out.canonical, "us.anthropic.claude-opus-4-7-v1:0");
    assert_eq!(out.wire_model_id, "us.anthropic.claude-opus-4-7-v1:0");
    assert!(out.opus_1m);
}
