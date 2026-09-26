//! Offline reproduction of the gateway's bounded prefetch replay handoff.
//! Takes a valid response.created SSE fixture and consumed prefetch byte count.
use aether_ai_formats::formats::shared::stream_rewrite::maybe_build_ai_surface_stream_rewriter;
use serde_json::{json, Value};
fn valid_data(body: &[u8]) -> bool {
    std::str::from_utf8(body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .all(|data| serde_json::from_str::<Value>(data).is_ok())
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = std::fs::read(&args[1]).unwrap();
    let consumed: usize = args[2].parse().unwrap();
    let context =
        json!({"provider_api_format":"openai:responses","client_api_format":"openai:responses"});
    let mut prefetch = maybe_build_ai_surface_stream_rewriter(Some(&context)).unwrap();
    let emitted = prefetch.push_chunk(&input[..consumed]).unwrap();
    let mut rebuilt = maybe_build_ai_surface_stream_rewriter(Some(&context)).unwrap();
    let _ = rebuilt.push_chunk(&input[..16384]).unwrap();
    let mut broken = emitted.clone();
    broken.extend(rebuilt.push_chunk(&input[consumed..]).unwrap());
    broken.extend(rebuilt.finish().unwrap());
    let expected = [&input[..16384], &input[consumed..]].concat();
    assert_eq!(broken, expected);
    assert!(!valid_data(&broken));
    let mut prefetch = prefetch.into_owned();
    drop(rebuilt);
    drop(context);
    let mut retained = emitted;
    retained.extend(prefetch.push_chunk(&input[consumed..]).unwrap());
    retained.extend(prefetch.finish().unwrap());
    assert_eq!(retained, input);
    assert!(valid_data(&retained));
    println!("REPRODUCED: bounded replay loses {} bytes at SSE offset 16384; retaining parser state preserves all {} bytes and valid JSON", consumed-16384, input.len());
}
