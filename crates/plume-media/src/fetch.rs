use crate::MediaError;

/// Nothing we show inline is bigger than this; anything larger is a link.
const MAX_BYTES: usize = 20 * 1024 * 1024;

pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, MediaError> {
    let response = http.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length()
        && length as usize > MAX_BYTES
    {
        return Err(MediaError::TooBig(length as usize));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_BYTES {
        return Err(MediaError::TooBig(bytes.len()));
    }
    Ok(bytes.to_vec())
}
