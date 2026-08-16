use std::{sync::OnceLock, time::Duration};

use reqwest::{Client, Response, Url, redirect::Policy};

use super::{WebError, security::public_headers};

pub(crate) async fn get_with_timeout(url: &Url, timeout_ms: u32) -> Result<Response, WebError> {
    static PUBLIC_CLIENT: OnceLock<Client> = OnceLock::new();
    let client = PUBLIC_CLIENT.get_or_init(|| {
        Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("public HTTP client must build")
    });
    let mut request = client
        .get(url.clone())
        .timeout(Duration::from_millis(timeout_ms as u64));
    for (name, value) in public_headers() {
        request = request.header(name, value);
    }
    Ok(request.send().await?)
}

pub(crate) async fn read_body_capped(
    mut response: Response,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool), WebError> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.chunk().await? {
        let remaining = max_bytes.saturating_sub(bytes.len());
        if remaining == 0 {
            truncated = true;
            break;
        }
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, truncated))
}

pub(crate) fn decode_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(crate) fn extract_title(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(content_start) = text[start..].find('>').map(|index| start + index + 1) else {
        return String::new();
    };
    let Some(end) = lower[content_start..]
        .find("</title>")
        .map(|index| content_start + index)
    else {
        return String::new();
    };
    strip_tags(&text[content_start..end])
}

fn strip_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}
