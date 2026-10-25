//! Sending files with a message (multipart/form-data).

use reqwest::multipart::{Form, Part};

/// Discord's limit for accounts without Nitro.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Upload {
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// The form Discord expects: the message as `payload_json`, then each file
/// as `files[N]`, matched up through `attachments[N].id`.
pub fn form(mut payload: serde_json::Value, files: &[Upload]) -> Form {
    payload["attachments"] = files
        .iter()
        .enumerate()
        .map(|(index, file)| serde_json::json!({ "id": index, "filename": file.filename }))
        .collect();
    let mut form = Form::new().text("payload_json", payload.to_string());
    for (index, file) in files.iter().enumerate() {
        let part = Part::bytes(file.bytes.clone()).file_name(file.filename.clone());
        form = form.part(format!("files[{index}]"), part);
    }
    form
}
