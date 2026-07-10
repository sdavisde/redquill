//! Pure JSON-RPC message construction, response normalization, and
//! `file://` URI conversion for the LSP client.
//!
//! Nothing in this module touches a process, socket, or async runtime: it
//! turns request parameters into serialized JSON-RPC payloads (ready to be
//! handed to [`crate::lsp::codec::encode_frame`]) and turns raw
//! `serde_json::Value` results back into the crate's own
//! [`SourceLocation`] / `String` types. `lsp_types::Uri` has no built-in
//! path conversion helpers (it is a thin newtype around `fluent_uri`), so
//! [`path_to_uri`] and [`uri_to_path`] implement `file://` percent-encoding
//! by hand.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

use crate::lsp::event::SourceLocation;

/// Percent-encodes an absolute path as a `file://` URI.
///
/// Returns `None` if the path is not absolute or is not valid UTF-8.
/// ASCII alphanumerics and `- . _ ~ /` are kept verbatim; every other byte
/// (including the UTF-8 bytes of non-ASCII characters) is percent-encoded.
pub(crate) fn path_to_uri(path: &Path) -> Option<Uri> {
    if !path.is_absolute() {
        return None;
    }
    let path_str = path.to_str()?;

    let mut encoded = String::with_capacity(path_str.len() + 7);
    encoded.push_str("file://");
    for byte in path_str.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            push_percent_encoded_byte(&mut encoded, byte);
        }
    }

    Uri::from_str(&encoded).ok()
}

/// Appends the percent-encoded form (`%XX`, uppercase hex) of `byte` to
/// `out`.
fn push_percent_encoded_byte(out: &mut String, byte: u8) {
    const HEX_DIGITS: [u8; 16] = *b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
    out.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
}

/// Parses a `file://` URI back to a path.
///
/// Returns `None` for non-`file` schemes, authorities other than empty or
/// `localhost` (case-insensitive), or invalid percent escapes.
pub(crate) fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let uri_str = uri.as_str();

    const PREFIX: &str = "file://";
    if uri_str.len() < PREFIX.len() {
        return None;
    }
    let (prefix, rest) = uri_str.split_at(PREFIX.len());
    if !prefix.eq_ignore_ascii_case(PREFIX) {
        return None;
    }

    let encoded_path = if rest.starts_with('/') {
        // Empty authority: file:///path
        rest
    } else {
        let slash_pos = rest.find('/')?;
        let (authority, path_with_slash) = rest.split_at(slash_pos);
        if !authority.eq_ignore_ascii_case("localhost") {
            return None;
        }
        path_with_slash
    };

    let decoded_bytes = percent_decode(encoded_path)?;
    let decoded_string = String::from_utf8(decoded_bytes).ok()?;
    Some(PathBuf::from(decoded_string))
}

/// Percent-decodes a string into raw bytes. Returns `None` on an
/// incomplete or non-hex `%XX` escape.
fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_value(bytes[i + 1])?;
            let lo = hex_value(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn build_request(id: u64, method: &str, params: serde_json::Value) -> Option<Vec<u8>> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    serde_json::to_vec(&message).ok()
}

fn build_notification(method: &str, params: serde_json::Value) -> Option<Vec<u8>> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    serde_json::to_vec(&message).ok()
}

fn text_document_position(
    uri: &Uri,
    line: u32,
    character: u32,
) -> lsp_types::TextDocumentPositionParams {
    lsp_types::TextDocumentPositionParams {
        text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
        position: lsp_types::Position { line, character },
    }
}

/// Builds an `initialize` request body.
///
/// `root_uri` is deprecated in favor of `workspace_folders` upstream, but
/// it remains the simplest single-root handshake and is what most servers
/// still key off of; hence the local `#[allow(deprecated)]`.
#[allow(deprecated)]
pub(crate) fn build_initialize(id: u64, root: &Path) -> Option<Vec<u8>> {
    let params = lsp_types::InitializeParams {
        process_id: Some(std::process::id()),
        root_uri: path_to_uri(root),
        capabilities: lsp_types::ClientCapabilities::default(),
        ..Default::default()
    };
    let params_value = serde_json::to_value(params).ok()?;
    build_request(id, "initialize", params_value)
}

/// Builds the `initialized` notification body sent once the server has
/// responded to `initialize`.
pub(crate) fn build_initialized() -> Option<Vec<u8>> {
    build_notification("initialized", serde_json::json!({}))
}

/// Builds a `textDocument/didOpen` notification body.
pub(crate) fn build_did_open(uri: &Uri, language_id: &str, text: &str) -> Option<Vec<u8>> {
    let params = lsp_types::DidOpenTextDocumentParams {
        text_document: lsp_types::TextDocumentItem {
            uri: uri.clone(),
            language_id: language_id.to_string(),
            version: 0,
            text: text.to_string(),
        },
    };
    let params_value = serde_json::to_value(params).ok()?;
    build_notification("textDocument/didOpen", params_value)
}

/// Builds a `textDocument/definition` request body.
pub(crate) fn build_definition(id: u64, uri: &Uri, line: u32, character: u32) -> Option<Vec<u8>> {
    let params = lsp_types::GotoDefinitionParams {
        text_document_position_params: text_document_position(uri, line, character),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    let params_value = serde_json::to_value(params).ok()?;
    build_request(id, "textDocument/definition", params_value)
}

/// Builds a `textDocument/references` request body. Always requests the
/// declaration itself be included (`include_declaration: true`).
pub(crate) fn build_references(id: u64, uri: &Uri, line: u32, character: u32) -> Option<Vec<u8>> {
    let params = lsp_types::ReferenceParams {
        text_document_position: text_document_position(uri, line, character),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: lsp_types::ReferenceContext {
            include_declaration: true,
        },
    };
    let params_value = serde_json::to_value(params).ok()?;
    build_request(id, "textDocument/references", params_value)
}

/// Builds a `textDocument/hover` request body.
pub(crate) fn build_hover(id: u64, uri: &Uri, line: u32, character: u32) -> Option<Vec<u8>> {
    let params = lsp_types::HoverParams {
        text_document_position_params: text_document_position(uri, line, character),
        work_done_progress_params: Default::default(),
    };
    let params_value = serde_json::to_value(params).ok()?;
    build_request(id, "textDocument/hover", params_value)
}

/// Builds a `shutdown` request body (`params: null`).
pub(crate) fn build_shutdown(id: u64) -> Option<Vec<u8>> {
    build_request(id, "shutdown", serde_json::Value::Null)
}

/// Builds the `exit` notification body. Unlike the other notifications
/// here, `exit` carries no `params` field at all.
pub(crate) fn build_exit() -> Option<Vec<u8>> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "exit",
    });
    serde_json::to_vec(&message).ok()
}

/// Builds a JSON-RPC response echoing `id` back with `result`, e.g. for
/// replying to server-to-client requests we don't otherwise act on.
pub(crate) fn build_null_reply(
    id: &serde_json::Value,
    result: serde_json::Value,
) -> Option<Vec<u8>> {
    let message = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    serde_json::to_vec(&message).ok()
}

fn location_to_source(location: &lsp_types::Location) -> Option<SourceLocation> {
    let path = uri_to_path(&location.uri)?;
    Some(SourceLocation {
        path,
        line: location.range.start.line,
        character: location.range.start.character,
    })
}

fn location_link_to_source(link: &lsp_types::LocationLink) -> Option<SourceLocation> {
    let path = uri_to_path(&link.target_uri)?;
    Some(SourceLocation {
        path,
        line: link.target_selection_range.start.line,
        character: link.target_selection_range.start.character,
    })
}

/// Normalizes a `textDocument/definition` response into a flat list of
/// [`SourceLocation`]s. Tolerates `null` (empty result) and all three
/// spec-legal shapes (`Location`, `Location[]`, `LocationLink[]`).
/// Non-`file` URIs are dropped rather than causing the whole response to
/// be discarded; malformed input yields an empty list.
pub(crate) fn normalize_definition(result: serde_json::Value) -> Vec<SourceLocation> {
    if result.is_null() {
        return Vec::new();
    }
    match serde_json::from_value::<lsp_types::GotoDefinitionResponse>(result) {
        Ok(lsp_types::GotoDefinitionResponse::Scalar(location)) => {
            location_to_source(&location).into_iter().collect()
        }
        Ok(lsp_types::GotoDefinitionResponse::Array(locations)) => {
            locations.iter().filter_map(location_to_source).collect()
        }
        Ok(lsp_types::GotoDefinitionResponse::Link(links)) => {
            links.iter().filter_map(location_link_to_source).collect()
        }
        Err(_) => Vec::new(),
    }
}

/// Normalizes a `textDocument/references` response (`Location[] | null`)
/// into a flat list of [`SourceLocation`]s. Malformed input yields an
/// empty list.
pub(crate) fn normalize_references(result: serde_json::Value) -> Vec<SourceLocation> {
    if result.is_null() {
        return Vec::new();
    }
    match serde_json::from_value::<Vec<lsp_types::Location>>(result) {
        Ok(locations) => locations.iter().filter_map(location_to_source).collect(),
        Err(_) => Vec::new(),
    }
}

fn marked_string_to_text(marked: &lsp_types::MarkedString) -> String {
    match marked {
        lsp_types::MarkedString::String(text) => text.clone(),
        lsp_types::MarkedString::LanguageString(language_string) => {
            format!(
                "```{}\n{}\n```",
                language_string.language, language_string.value
            )
        }
    }
}

/// Converts a single (non-array) `contents` JSON value into text, trying
/// [`lsp_types::MarkedString`] before [`lsp_types::MarkupContent`] — the
/// same priority `HoverContents`'s `Scalar`-before-`Markup` variant order
/// implies.
fn scalar_contents_to_text(value: serde_json::Value) -> Option<String> {
    if let Ok(marked) = serde_json::from_value::<lsp_types::MarkedString>(value.clone()) {
        return Some(marked_string_to_text(&marked));
    }
    let markup: lsp_types::MarkupContent = serde_json::from_value(value).ok()?;
    Some(markup.value)
}

/// Normalizes a `textDocument/hover` response into a single displayable
/// string. Tolerates `null` and all three spec-legal `contents` shapes.
/// Array entries are joined with a blank line, skipping empty entries.
/// Returns `None` for `null`, malformed input, or content that trims to
/// empty.
///
/// `contents` is dispatched on the *raw* JSON shape (array vs. scalar)
/// rather than deserialized straight into [`lsp_types::HoverContents`]:
/// serde's derived struct `Deserialize` also accepts a JSON array as a
/// positional field list, so an untagged `HoverContents` would silently
/// misparse a two-string `contents` array (`["one", "two"]`) as a single
/// `MarkedString::LanguageString { language: "one", value: "two" }`
/// instead of two separate entries. Branching on the JSON shape first
/// avoids that ambiguity; each individual array element is still scalar
/// JSON (a string or an object), so deserializing it alone as a
/// `MarkedString` is unambiguous.
pub(crate) fn normalize_hover(result: serde_json::Value) -> Option<String> {
    if result.is_null() {
        return None;
    }
    let contents = result.get("contents")?.clone();
    let text = match contents {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value::<lsp_types::MarkedString>(item).ok())
            .map(|marked| marked_string_to_text(&marked))
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        scalar => scalar_contents_to_text(scalar)?,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn path_to_uri_roundtrip_with_space_and_unicode() {
        let path = Path::new("/tmp/some file/\u{3b1}.rs");
        let uri = path_to_uri(path).expect("absolute utf8 path should encode");
        assert_eq!(uri.as_str(), "file:///tmp/some%20file/%CE%B1.rs");

        let round_tripped = uri_to_path(&uri).expect("uri should decode back to a path");
        assert_eq!(round_tripped, path);
    }

    #[test]
    fn path_to_uri_rejects_relative_paths() {
        assert_eq!(path_to_uri(Path::new("relative/path.rs")), None);
    }

    #[test]
    fn uri_to_path_rejects_non_file_scheme() {
        let uri = Uri::from_str("https://example.com/path").expect("valid uri");
        assert_eq!(uri_to_path(&uri), None);
    }

    #[test]
    fn uri_to_path_accepts_localhost_authority() {
        let uri = Uri::from_str("file://localhost/tmp/x").expect("valid uri");
        assert_eq!(uri_to_path(&uri), Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn uri_to_path_rejects_other_authority() {
        let uri = Uri::from_str("file://otherhost/tmp/x").expect("valid uri");
        assert_eq!(uri_to_path(&uri), None);
    }

    #[test]
    fn uri_to_path_rejects_invalid_escape() {
        // `%zz` is not even syntactically valid percent-encoding (the hex
        // digits themselves are invalid), so `fluent_uri` rejects it before
        // we ever see a `Uri` to decode — exercise that hex-digit rejection
        // directly against `percent_decode` instead.
        assert_eq!(percent_decode("/tmp/%zz"), None);

        // `%FF` is syntactically valid percent-encoding (valid hex digits)
        // but decodes to a lone byte that is not valid UTF-8 on its own, so
        // it reaches `uri_to_path`'s `String::from_utf8` guard and is
        // rejected there.
        let uri = Uri::from_str("file:///tmp/%FF").expect("valid uri");
        assert_eq!(uri_to_path(&uri), None);
    }

    #[test]
    fn normalize_definition_null_is_empty() {
        assert_eq!(normalize_definition(json!(null)), Vec::new());
    }

    #[test]
    fn normalize_definition_single_location() {
        let result = json!({
            "uri": "file:///tmp/a.rs",
            "range": {
                "start": { "line": 3, "character": 5 },
                "end": { "line": 3, "character": 9 }
            }
        });
        let locations = normalize_definition(result);
        assert_eq!(
            locations,
            vec![SourceLocation {
                path: PathBuf::from("/tmp/a.rs"),
                line: 3,
                character: 5,
            }]
        );
    }

    #[test]
    fn normalize_definition_array_of_locations() {
        let result = json!([
            {
                "uri": "file:///tmp/a.rs",
                "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 1 } }
            },
            {
                "uri": "file:///tmp/b.rs",
                "range": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 1 } }
            }
        ]);
        let locations = normalize_definition(result);
        assert_eq!(
            locations,
            vec![
                SourceLocation {
                    path: PathBuf::from("/tmp/a.rs"),
                    line: 1,
                    character: 0,
                },
                SourceLocation {
                    path: PathBuf::from("/tmp/b.rs"),
                    line: 2,
                    character: 0,
                },
            ]
        );
    }

    #[test]
    fn normalize_definition_array_of_location_links_uses_target_selection_range() {
        let result = json!([
            {
                "targetUri": "file:///tmp/a.rs",
                "targetRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 5, "character": 0 } },
                "targetSelectionRange": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 8 } }
            }
        ]);
        let locations = normalize_definition(result);
        assert_eq!(
            locations,
            vec![SourceLocation {
                path: PathBuf::from("/tmp/a.rs"),
                line: 2,
                character: 4,
            }]
        );
    }

    #[test]
    fn normalize_definition_malformed_is_empty() {
        assert_eq!(normalize_definition(json!({"garbage": true})), Vec::new());
        assert_eq!(normalize_definition(json!("not an object")), Vec::new());
    }

    #[test]
    fn normalize_references_null_is_empty() {
        assert_eq!(normalize_references(json!(null)), Vec::new());
    }

    #[test]
    fn normalize_references_array() {
        let result = json!([
            {
                "uri": "file:///tmp/a.rs",
                "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 1 } }
            }
        ]);
        assert_eq!(
            normalize_references(result),
            vec![SourceLocation {
                path: PathBuf::from("/tmp/a.rs"),
                line: 1,
                character: 0,
            }]
        );
    }

    #[test]
    fn normalize_hover_null_is_none() {
        assert_eq!(normalize_hover(json!(null)), None);
    }

    #[test]
    fn normalize_hover_plain_string() {
        let result = json!({ "contents": "hello" });
        assert_eq!(normalize_hover(result), Some("hello".to_string()));
    }

    #[test]
    fn normalize_hover_language_string_is_fenced() {
        let result = json!({ "contents": { "language": "rust", "value": "fn main() {}" } });
        assert_eq!(
            normalize_hover(result),
            Some("```rust\nfn main() {}\n```".to_string())
        );
    }

    #[test]
    fn normalize_hover_array_joined_by_blank_line() {
        let result = json!({ "contents": ["one", "two"] });
        assert_eq!(normalize_hover(result), Some("one\n\ntwo".to_string()));
    }

    #[test]
    fn normalize_hover_markup_content() {
        let result = json!({ "contents": { "kind": "markdown", "value": "**bold**" } });
        assert_eq!(normalize_hover(result), Some("**bold**".to_string()));
    }

    #[test]
    fn normalize_hover_empty_string_is_none() {
        let result = json!({ "contents": "" });
        assert_eq!(normalize_hover(result), None);
    }

    #[test]
    fn build_definition_round_trips_through_json() {
        let uri = Uri::from_str("file:///tmp/a.rs").expect("valid uri");
        let bytes = build_definition(42, &uri, 7, 3).expect("should serialize");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");
        assert_eq!(value["method"], json!("textDocument/definition"));
        assert_eq!(value["id"], json!(42));
        assert_eq!(value["params"]["position"]["line"], json!(7));
        assert_eq!(value["params"]["position"]["character"], json!(3));
    }

    #[test]
    fn build_null_reply_echoes_ids_verbatim() {
        let bytes = build_null_reply(&json!("string-id"), json!(null)).expect("should serialize");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");
        assert_eq!(value["id"], json!("string-id"));

        let bytes = build_null_reply(&json!(7), json!(null)).expect("should serialize");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json");
        assert_eq!(value["id"], json!(7));
    }
}
