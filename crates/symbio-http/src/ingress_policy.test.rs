//! 共享入站策略测试：白名单与幂等键表是本模块自己的不变量。

use super::*;

use serde_json::json;

#[test]
fn readonly_allowlist() {
    let none = json!({});
    let at = |rel: &str| json!({ "path": format!("@vfs/{rel}") });

    assert!(is_readonly_allowed("vdfs/list", &none));
    assert!(is_readonly_allowed("vdfs/root", &none));
    assert!(is_readonly_allowed("vdfs/read", &at("session/abc")));
    assert!(is_readonly_allowed("vdfs/search", &at("model")));

    assert!(!is_readonly_allowed("home/get_homedir", &none));
    assert!(!is_readonly_allowed("home/get_workspace", &none));
    assert!(!is_readonly_allowed("vdfs/write", &none));
    assert!(!is_readonly_allowed("vdfs/delete", &none));
    assert!(!is_readonly_allowed("vdfs/action", &none));
    assert!(!is_readonly_allowed("session/stats", &none));
    assert!(!is_readonly_allowed("bogus/path", &none));

    assert!(!is_readonly_allowed("vdfs/read", &at("gateway/PLUGIN.yml")));
    assert!(!is_readonly_allowed("vdfs/read", &at("web/PLUGIN.yml")));
    assert!(is_readonly_allowed("vdfs/stat", &at("web/PLUGIN.yml")));
    assert!(is_readonly_allowed("vdfs/read", &at("web")));
}

#[test]
fn idempotency_rejects_duplicates_until_released() {
    let table = Idempotency::new();
    let ticket = table.reserve("k-1").expect("first reservation");
    assert!(table.reserve("k-1").is_none());
    assert!(table.reserve("k-2").is_some());
    ticket.release();
    assert!(table.reserve("k-1").is_some());
}

#[test]
fn idempotency_forgets_entries_past_ttl() {
    let table = Idempotency::new();
    let stored = Instant::now();
    table
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert("stale".to_string(), stored);
    assert!(table
        .reserve_at("stale", stored + IDEMPOTENCY_TTL + Duration::from_secs(1))
        .is_some());
}

#[test]
fn idempotency_evicts_oldest_when_full() {
    let table = Idempotency::new();
    let base = Instant::now();
    {
        let mut seen = table.seen.lock().unwrap_or_else(|error| error.into_inner());
        for index in 0..IDEMPOTENCY_CAPACITY - 1 {
            seen.insert(format!("old-{index}"), base + Duration::from_secs(10));
        }
        seen.insert("oldest".to_string(), base);
    }
    assert!(table
        .reserve_at("fresh", base + Duration::from_secs(500))
        .is_some());
    let seen = table.seen.lock().unwrap_or_else(|error| error.into_inner());
    assert_eq!(seen.len(), IDEMPOTENCY_CAPACITY);
    assert!(!seen.contains_key("oldest"));
}

#[test]
fn idempotency_key_shape_is_bounded() {
    assert!(valid_idempotency_key(
        "550e8400-e29b-41d4-a716-446655440000"
    ));
    assert!(!valid_idempotency_key(""));
    assert!(!valid_idempotency_key("has space"));
    assert!(!valid_idempotency_key("line\nbreak"));
    assert!(!valid_idempotency_key(
        &"a".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1)
    ));
}

fn request_with_key(key: Option<&str>) -> HttpRequest {
    let mut headers = HashMap::new();
    if let Some(key) = key {
        headers.insert("idempotency-key".to_string(), key.to_string());
    }
    HttpRequest {
        method: "POST".to_string(),
        path: "/api/v1/invoke".to_string(),
        headers,
        body: Vec::new(),
    }
}

#[test]
fn reserve_idempotency_maps_invalid_and_duplicate_keys_to_http_errors() {
    let table = Idempotency::new();
    assert!(reserve_idempotency(&request_with_key(None), &table)
        .expect("absent key")
        .is_none());
    assert_eq!(
        reserve_idempotency(&request_with_key(Some("bad key")), &table)
            .expect_err("invalid key")
            .status,
        400
    );

    let ticket = reserve_idempotency(&request_with_key(Some("same")), &table)
        .expect("first key")
        .expect("reserved");
    assert_eq!(
        reserve_idempotency(&request_with_key(Some("same")), &table)
            .expect_err("duplicate key")
            .status,
        409
    );
    ticket.release();
    assert!(reserve_idempotency(&request_with_key(Some("same")), &table)
        .expect("released key")
        .is_some());
}
