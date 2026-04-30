use serde_json::json;
use warp_byok_proxy::cache::{apply_cache_points, CacheInputs};

#[test]
fn cache_disabled_no_changes() {
    let msgs = vec![json!({"role":"user","content":[{"type":"text","text":"hi"}]})];
    let sys = Some(json!([{"type":"text","text":"system prompt"}]));
    let r = apply_cache_points(CacheInputs {
        enabled: false,
        messages: msgs.clone(),
        system: sys.clone(),
    });
    assert_eq!(r.messages, msgs);
    assert_eq!(r.system, sys);
}

#[test]
fn cache_enabled_appends_cachepoint_to_system() {
    let sys = Some(json!([{"type":"text","text":"system"}]));
    let r = apply_cache_points(CacheInputs {
        enabled: true,
        messages: vec![],
        system: sys,
    });
    let s = r.system.unwrap();
    assert_eq!(
        s[s.as_array().unwrap().len() - 1],
        json!({"cachePoint":{"type":"default"}})
    );
}

#[test]
fn cache_enabled_tags_last_two_user_messages() {
    let msgs = vec![
        json!({"role":"user","content":[{"type":"text","text":"u1"}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"a1"}]}),
        json!({"role":"user","content":[{"type":"text","text":"u2"}]}),
        json!({"role":"assistant","content":[{"type":"text","text":"a2"}]}),
        json!({"role":"user","content":[{"type":"text","text":"u3"}]}),
    ];
    let r = apply_cache_points(CacheInputs {
        enabled: true,
        messages: msgs,
        system: None,
    });
    // u2 and u3 should have cachePoint appended
    let u2 = &r.messages[2]["content"];
    let u3 = &r.messages[4]["content"];
    assert!(u2
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c.get("cachePoint").is_some()));
    assert!(u3
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c.get("cachePoint").is_some()));
    // u1 should NOT
    let u1 = &r.messages[0]["content"];
    assert!(u1
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c.get("cachePoint").is_none()));
}
