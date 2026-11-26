//! Pictures on the Stats page: rank badges, item icons and Steam avatars.
//!
//! Three hosts serve them, and only these three (owner approved 2026-11-26, read only while
//! Player stats is on):
//!
//! - `api.deadlock-api.com`: rank badge art (`/v1/assets/ranks/{rank}/{subrank}/image`).
//! - `assets-bucket.deadlock-api.com`: item icons, the URLs the item asset response names.
//! - `avatars.steamstatic.com`: Steam avatars, the URLs `/v1/players/steam` names.
//!
//! A request carries the image path and nothing else about the player: no cookie, no key, no
//! account id beyond what a public avatar URL already is. Any other host is refused here
//! before a socket opens, so a URL from an API response can never send Sigil elsewhere.

use crate::error::{Error, Result};

/// Every host a Stats picture may come from.
pub const IMAGE_HOSTS: [&str; 3] = [
    "api.deadlock-api.com",
    "assets-bucket.deadlock-api.com",
    "avatars.steamstatic.com",
];

/// The most bytes one picture may have. A badge is about 200 KB, an icon or avatar far less.
pub const IMAGE_BYTES: u64 = 1024 * 1024;

/// True for an `https://` URL on one of [`IMAGE_HOSTS`], with no user info or port.
pub fn is_allowed_image_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') || authority.contains(':') {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    IMAGE_HOSTS.contains(&host.as_str())
}

/// The badge art for a rank tier and subrank, on the API host.
pub fn rank_badge_url(rank: u32, subrank: u32) -> String {
    format!("{}/v1/assets/ranks/{rank}/{subrank}/image?format=webp", super::STATS_API_BASE)
}

/// The rank tier and subrank behind a badge number (`113` is tier 11, subrank 3).
pub fn badge_parts(badge: u32) -> Option<(u32, u32)> {
    let (rank, subrank) = (badge / 10, badge % 10);
    (rank > 0 && (1..=6).contains(&subrank)).then_some((rank, subrank))
}

/// A file name for one picture in the art cache: the URL's FNV-1a hash, so the cache never
/// holds a name or id in clear and two URLs never share a file by accident.
pub fn cache_name(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Fetch one picture's bytes. Refuses a URL off the allowlist before a socket opens, never
/// follows a redirect (an allowed host must not be able to send the request elsewhere), and
/// takes only a 2xx answer with an `image/` content type and at most [`IMAGE_BYTES`] of body.
/// Goes through the offline gate like every core request.
pub fn fetch_image(url: &str) -> Result<Vec<u8>> {
    fetch_with(url, is_allowed_image_url)
}

fn fetch_with(url: &str, allowed: fn(&str) -> bool) -> Result<Vec<u8>> {
    if !allowed(url) {
        return Err(Error::msg(format!("Refused a picture from a host Sigil does not use: {url}")));
    }
    let agent = crate::net::builder()
        .middleware(crate::net::gate)
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", super::STATS_USER_AGENT)
        .call()
        .map_err(|error| Error::Network(format!("Picture request failed: {error}")))?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        let to = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        return Err(Error::Network(format!("Refused a picture redirect ({status}) to {to}")));
    }
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("Picture request returned HTTP {status}.")));
    }
    let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    if !kind.starts_with("image/") {
        return Err(Error::Network(format!("Refused a picture that is not an image ({kind}).")));
    }
    response
        .body_mut()
        .with_config()
        .limit(IMAGE_BYTES)
        .read_to_vec()
        .map_err(|error| Error::Network(format!("Picture could not be read: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_hosts_pass() {
        assert!(is_allowed_image_url("https://api.deadlock-api.com/v1/assets/ranks/11/3/image?format=webp"));
        assert!(is_allowed_image_url("https://assets-bucket.deadlock-api.com/assets-api-res/images/items/weapon/basic_magazine.webp"));
        assert!(is_allowed_image_url("https://avatars.steamstatic.com/7225d9a702f3e4804cac7eaf932384e53747028d_medium.jpg"));
        assert!(is_allowed_image_url("https://AVATARS.steamstatic.com/a.jpg"));
        for bad in [
            "http://avatars.steamstatic.com/a.jpg",
            "https://avatars.steamstatic.com.evil.example/a.jpg",
            "https://evil.example/https://avatars.steamstatic.com/a.jpg",
            "https://user@avatars.steamstatic.com/a.jpg",
            "https://avatars.steamstatic.com:8443/a.jpg",
            "https://steamcdn-a.akamaihd.net/a.jpg",
            "",
        ] {
            assert!(!is_allowed_image_url(bad), "{bad}");
        }
    }

    #[test]
    fn a_refused_url_never_reaches_the_network() {
        crate::net::TEST_OFFLINE.with(|offline| offline.set(false));
        let error = fetch_image("https://evil.example/a.png").unwrap_err().to_string();
        assert!(error.contains("Refused"), "{error}");
    }

    /// One canned HTTP answer on a local port, for the fetch rules.
    fn serve(answer: &'static [u8]) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(answer);
            }
        });
        format!("http://127.0.0.1:{port}/a.png")
    }

    fn local(url: &str) -> bool {
        url.starts_with("http://127.0.0.1:")
    }

    #[test]
    fn a_redirect_is_never_followed() {
        let url = serve(b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/a.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let error = fetch_with(&url, local).unwrap_err().to_string();
        assert!(error.contains("Refused a picture redirect") && error.contains("evil.example"), "{error}");
    }

    #[test]
    fn only_an_image_within_the_cap_is_taken() {
        let url = serve(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\nConnection: close\r\n\r\nhtml");
        assert!(fetch_with(&url, local).unwrap_err().to_string().contains("not an image"));
        let url = serve(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!");
        assert_eq!(fetch_with(&url, local).unwrap(), b"PNG!");
        let big: &'static [u8] = Box::leak(
            [format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", IMAGE_BYTES + 10).into_bytes(), vec![0u8; IMAGE_BYTES as usize + 10]]
                .concat()
                .into_boxed_slice(),
        );
        assert!(fetch_with(&serve(big), local).is_err(), "over the cap");
    }

    #[test]
    fn badges_split_into_rank_and_subrank() {
        assert_eq!(badge_parts(113), Some((11, 3)));
        assert_eq!(badge_parts(16), Some((1, 6)));
        assert_eq!(badge_parts(0), None);
        assert_eq!(badge_parts(110), None);
        assert!(rank_badge_url(11, 3).ends_with("/v1/assets/ranks/11/3/image?format=webp"));
        assert!(is_allowed_image_url(&rank_badge_url(11, 3)));
    }

    #[test]
    fn cache_names_are_stable_hex() {
        let a = cache_name("https://avatars.steamstatic.com/a.jpg");
        assert_eq!(a.len(), 16);
        assert_eq!(a, cache_name("https://avatars.steamstatic.com/a.jpg"));
        assert_ne!(a, cache_name("https://avatars.steamstatic.com/b.jpg"));
    }
}
