// Confirms the warp_multi_agent_api protobuf crate exposes the types the proxy needs.
// Type path note: prost emits Request and ResponseEvent at the crate root of
// warp_multi_agent_api (verified via target/debug/build/warp_multi_agent_api-*/out/*.rs).
// If upstream reorganizes and these tests fail to compile, inspect the generated
// modules under target/debug/build/warp_multi_agent_api-*/out/ for the new path.

#[test]
fn can_construct_minimal_request() {
    // Just assert the types compile and a default Request can be built.
    let _req: warp_multi_agent_api::Request = warp_multi_agent_api::Request::default();
}

#[test]
fn response_event_oneof_is_present() {
    // ResponseEvent should have StreamInit / ClientActions / StreamFinished variants.
    let _re: warp_multi_agent_api::ResponseEvent = warp_multi_agent_api::ResponseEvent::default();
}
