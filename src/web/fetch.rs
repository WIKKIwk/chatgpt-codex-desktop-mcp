use serde::Serialize;

use crate::config::Config;

use super::{
    WebError,
    http::{decode_utf8, extract_title, get_with_timeout, read_body_capped},
    security::assert_public_url,
};

#[derive(Debug, Clone, Serialize)]
pub struct FetchResult {
    #[serde(rename = "finalUrl")]
    pub final_url: String,
    pub status: u16,
    #[serde(rename = "contentType")]
    pub content_type: String,
    pub title: String,
    pub text: String,
    pub truncated: bool,
}

pub async fn web_fetch(config: &Config, input_url: &str) -> Result<FetchResult, WebError> {
    if !config.web_tools_enabled {
        return Err(WebError::Disabled);
    }
    let mut current = assert_public_url(input_url).await?;
    for _redirect_count in 0..=5 {
        let response = get_with_timeout(&current, config.web_timeout_ms).await?;
        if response.status().is_redirection() {
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(WebError::RedirectLocationMissing(status))?;
            let next = current
                .join(location)
                .map_err(|error| WebError::InvalidUrl(error.to_string()))?;
            current = assert_public_url(next.as_str()).await?;
            continue;
        }

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let (bytes, truncated) = read_body_capped(response, config.web_max_bytes as usize).await?;
        let text = decode_utf8(&bytes);
        return Ok(FetchResult {
            final_url: current.to_string(),
            status,
            content_type,
            title: extract_title(&text),
            text,
            truncated,
        });
    }
    Err(WebError::TooManyRedirects)
}
