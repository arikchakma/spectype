use spectype_document::HostError;

/// A document a loader returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDocument {
    /// The document's text.
    pub text: String,
    /// The `Content-Type` of an http(s) response. Like Redocly, a referenced document is parsed
    /// as JSON/YAML when its ref ends in `.json`/`.yml`/`.yaml` or this mentions `json`, `yaml`
    /// or `openapi`; otherwise it is kept as a string.
    pub mime_type: Option<String>,
}

/// Loads a document by absolute reference: a file path or an `http(s)` URL. `Ok(None)` means it
/// does not exist. Either failure leaves the `$ref` unresolved, as with Redocly.
pub type Load<'a> = dyn FnMut(&str) -> Result<Option<LoadedDocument>, HostError> + 'a;

/// Reads a file like Redocly's `BaseResolver`: `Ok(None)` when it does not exist, an error for a
/// directory.
pub fn load_file(path: &str) -> Result<Option<LoadedDocument>, HostError> {
    let io_error = |e: std::io::Error| HostError::with_source(format!("{e}, open '{path}'"), e);
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
            Ok(Some(LoadedDocument { text, mime_type: None }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) => {
            Err(HostError::new(format!("Expected a file but received a folder at {path}.")))
        }
        Err(e) => Err(io_error(e)),
    }
}
