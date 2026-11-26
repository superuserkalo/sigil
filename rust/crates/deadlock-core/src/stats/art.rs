//! Pictures on the Stats page: rank badges.
//!
//! Only `api.deadlock-api.com` serves them (`/v1/assets/ranks/{rank}/{subrank}/image`), read
//! only while Player stats is on. A request carries the image path and nothing else about the
//! player. Any other host is refused here before a socket opens, so a URL from an API response
//! can never send Sigil elsewhere.

use crate::error::{Error, Result};

/// Every host a Stats picture may come from.
pub const IMAGE_HOSTS: [&str; 1] = ["api.deadlock-api.com"];

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

/// Fetch one picture's bytes. Refuses a URL off the allowlist and a body over
/// [`IMAGE_BYTES`]; goes through the offline gate like every core request.
pub fn fetch_image(url: &str) -> Result<Vec<u8>> {
    if !is_allowed_image_url(url) {
        return Err(Error::msg(format!("Refused a picture from a host Sigil does not use: {url}")));
    }
    let agent = crate::net::agent(std::time::Duration::from_secs(20));
    let mut response = agent
        .get(url)
        .header("User-Agent", super::STATS_USER_AGENT)
        .call()
        .map_err(|error| Error::Network(format!("Picture request failed: {error}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("Picture request returned HTTP {status}.")));
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
    fn only_the_api_host_passes() {
        assert!(is_allowed_image_url("https://api.deadlock-api.com/v1/assets/ranks/11/3/image?format=webp"));
        assert!(is_allowed_image_url("https://API.deadlock-api.com/v1/assets/ranks/1/1/image"));
        for bad in [
            "https://avatars.steamstatic.com/a.jpg",
            "http://api.deadlock-api.com/a.jpg",
            "https://api.deadlock-api.com.evil.example/a.jpg",
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
