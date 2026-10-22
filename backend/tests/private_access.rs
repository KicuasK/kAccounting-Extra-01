use axum::http::{HeaderMap, HeaderValue, Method};
use kaccounting_backend::private_access::{
    PrivateAccessError, PrivateAccessPolicy, TrustedIngress,
};
fn policy() -> PrivateAccessPolicy {
    PrivateAccessPolicy::new("book.example.ts.net", "owner@example.com").unwrap()
}
fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("host", HeaderValue::from_static("book.example.ts.net"));
    h.insert(
        "origin",
        HeaderValue::from_static("https://book.example.ts.net"),
    );
    h.insert(
        "tailscale-user-login",
        HeaderValue::from_static("owner@example.com"),
    );
    h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    h
}
#[test]
fn spoofed_identity_on_regular_loopback_is_always_rejected() {
    assert_eq!(
        policy().authorize(&headers(), &Method::POST, TrustedIngress::RegularLoopback),
        Err(PrivateAccessError::UntrustedIngress)
    );
}
#[test]
fn canonical_private_policy_requires_exact_own_identity_host_and_origin() {
    assert!(
        policy()
            .authorize(
                &headers(),
                &Method::POST,
                TrustedIngress::AuthenticatedPrivateProxy
            )
            .is_ok()
    );
    for (name, bad) in [
        ("host", "book.example.ts.net:443"),
        ("host", "evil.ts.net"),
        ("origin", "https://book.example.ts.net/"),
        ("origin", "null"),
        ("tailscale-user-login", "other@example.com"),
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-site", "same-site"),
    ] {
        let mut h = headers();
        h.insert(name, HeaderValue::from_str(bad).unwrap());
        assert!(
            policy()
                .authorize(&h, &Method::POST, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err(),
            "{name} {bad}"
        );
    }
}
#[test]
fn duplicate_headers_and_forwarding_overrides_are_rejected() {
    for name in ["host", "origin", "tailscale-user-login", "sec-fetch-site"] {
        let mut h = headers();
        let v = h[name].clone();
        h.append(name, v);
        assert!(
            policy()
                .authorize(&h, &Method::GET, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
    for name in [
        "forwarded",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-original-host",
        "x-original-url",
    ] {
        let mut h = headers();
        h.insert(name, HeaderValue::from_static("ignored"));
        assert!(
            policy()
                .authorize(&h, &Method::GET, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
}
#[test]
fn navigation_can_omit_origin_but_mutations_cannot() {
    let mut h = headers();
    h.remove("origin");
    h.insert("sec-fetch-site", HeaderValue::from_static("none"));
    assert!(
        policy()
            .authorize(&h, &Method::GET, TrustedIngress::AuthenticatedPrivateProxy)
            .is_ok()
    );
    for m in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ] {
        assert!(
            policy()
                .authorize(&h, &m, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
}
#[test]
fn invalid_configuration_is_rejected() {
    for host in [
        "example.ts.net",
        "BOOK.example.ts.net",
        "book.example.ts.net.",
        "book.example.ts.net:443",
        "book.example.ts.net.evil",
        "https://book.example.ts.net",
        "book..example.ts.net",
        "-book.example.ts.net",
        "book_example.ts.net",
        "127.0.0.1",
    ] {
        assert!(
            PrivateAccessPolicy::new(host, "owner@example.com").is_err(),
            "{host}"
        );
    }
    for login in [
        "",
        "owner",
        "owner@",
        "@example.com",
        "owner@example.com\n",
        "owner@example.com,other@example.com",
        "owner@EXAMPLE.com",
    ] {
        assert!(
            PrivateAccessPolicy::new("book.example.ts.net", login).is_err(),
            "{login}"
        );
    }
}

#[test]
fn missing_nontext_and_combined_security_values_are_rejected() {
    for name in ["host", "tailscale-user-login"] {
        let mut h = headers();
        h.remove(name);
        assert!(
            policy()
                .authorize(&h, &Method::GET, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
        h.insert(name, HeaderValue::from_bytes(&[0xff]).unwrap());
        assert!(
            policy()
                .authorize(&h, &Method::GET, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
    for (name, value) in [
        ("host", "book.example.ts.net, evil.ts.net"),
        ("origin", "https://book.example.ts.net https://evil.test"),
        (
            "tailscale-user-login",
            "owner@example.com,other@example.com",
        ),
        ("sec-fetch-site", "same-origin, cross-site"),
    ] {
        let mut h = headers();
        h.insert(name, HeaderValue::from_str(value).unwrap());
        assert!(
            policy()
                .authorize(&h, &Method::POST, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
}

#[test]
fn github_provider_login_is_validated_but_never_normalized_or_aliased() {
    let policy = PrivateAccessPolicy::new("book.example.ts.net", "FakeOwner@github").unwrap();
    let mut h = headers();
    h.insert(
        "tailscale-user-login",
        HeaderValue::from_static("FakeOwner@github"),
    );
    assert!(
        policy
            .authorize(&h, &Method::POST, TrustedIngress::AuthenticatedPrivateProxy)
            .is_ok()
    );
    for bad in [
        "fakeowner@github",
        "FakeOwner@GITHUB",
        "FakeOwner@github.com",
        "other@github",
    ] {
        h.insert("tailscale-user-login", HeaderValue::from_str(bad).unwrap());
        assert!(
            policy
                .authorize(&h, &Method::POST, TrustedIngress::AuthenticatedPrivateProxy)
                .is_err()
        );
    }
    for bad in [
        "owner@local",
        "owner@localhost",
        "owner@GITHUB",
        "owner@gitlab",
        "owner@github.",
        "owner@@github",
        "owner@github\n",
        "owner@-github",
    ] {
        assert!(
            PrivateAccessPolicy::new("book.example.ts.net", bad).is_err(),
            "{bad}"
        );
    }
}
