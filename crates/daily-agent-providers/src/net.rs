/// Formats an error with its `source()` chain; reqwest hides the underlying cause
/// (e.g. "connection reset", "unexpected EOF") behind "error decoding response body".
pub(crate) fn describe_error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        message.push_str(&format!(": {next}"));
        source = next.source();
    }
    message
}
