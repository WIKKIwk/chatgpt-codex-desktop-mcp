use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::{Url, header::HeaderValue};
use tokio::net::lookup_host;

use super::WebError;

pub(crate) async fn assert_public_url(input: &str) -> Result<Url, WebError> {
    let url = input
        .parse::<Url>()
        .map_err(|error| WebError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::InvalidFetchProtocol);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebError::FetchUrlCredentials);
    }
    let host = url
        .host_str()
        .ok_or(WebError::MissingHost)?
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(WebError::LocalhostBlocked);
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        if is_private_address(address) {
            return Err(WebError::PrivateAddressBlocked);
        }
        return Ok(url);
    }

    let port = url.port_or_known_default().ok_or(WebError::MissingHost)?;
    let addresses = lookup_host((host.as_str(), port))
        .await
        .map_err(|_| WebError::HostResolution(host.clone()))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(WebError::HostResolution(host));
    }
    if addresses
        .iter()
        .any(|address| is_private_address(address.ip()))
    {
        return Err(WebError::PrivateAddressBlocked);
    }
    Ok(url)
}

pub(crate) fn make_search_url(base: &str) -> Result<Url, WebError> {
    let mut url = base
        .parse::<Url>()
        .map_err(|error| WebError::InvalidUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::InvalidSearchUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebError::SearchUrlCredentials);
    }
    let path = url.path().trim_end_matches('/');
    url.set_path(&format!("{path}/search"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn public_headers() -> [(reqwest::header::HeaderName, HeaderValue); 2] {
    [
        (
            reqwest::header::USER_AGENT,
            HeaderValue::from_static("chatgpt-codex-tools-mcp/0.1"),
        ),
        (
            reqwest::header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/json,text/plain,application/xml;q=0.9,*/*;q=0.5",
            ),
        ),
    ]
}

pub(crate) fn is_private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_private_ipv4(address),
        IpAddr::V6(address) => {
            address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || address.is_unicast_link_local()
                || is_private_ipv6(address)
                || address.to_ipv4_mapped().is_some_and(is_private_ipv4)
        }
    }
}

fn is_private_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 198 && (18..=19).contains(&b))
        || a >= 224
}

fn is_private_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
}
