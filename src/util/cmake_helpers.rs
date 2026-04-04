use std::path::Path;

/// Convert paths to a form that plays nicely with CMake and Windows shells.
/// - Strip leading verbatim prefix (`\\?\` or `//?/`) if present (CMake 4.1+ often uses this).
/// - On Windows, return backslashes `\`.
/// - On non-Windows, return forward slashes `/`.
pub fn normalize_path<P: AsRef<Path>>(p: P) -> String {
    let mut s = p.as_ref().to_string_lossy().into_owned();

    // Strip Windows verbatim prefixes if present
    if s.starts_with(r"\\?\") {
        // remove the leading \\?\
        s = s.replacen(r"\\?\", "", 1);
    } else if s.starts_with("//?/") {
        // remove the leading //?/
        s = s.replacen("//?/", "", 1);
    }

    // Normalize separators per-platform
    if cfg!(windows) {
        // Use backslashes on Windows
        s = s.replace('/', r"\");
    } else {
        // Use forward slashes elsewhere
        s = s.replace('\\', "/");
    }

    s
}

pub fn cmake_quote(val: &str) -> String {
    let s = val.trim().replace('"', "\\\"");
    format!("\"{}\"", s)
}

pub fn infer_cmake_type(val: &str) -> &'static str {
    match val.to_ascii_uppercase().as_str() {
        "ON" | "OFF" | "TRUE" | "FALSE" | "YES" | "NO" => "BOOL",
        _ => "STRING",
    }
}

pub fn split_kv(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find('=') {
        let (k, v) = raw.split_at(idx);
        let key = k.trim().to_string();
        let mut val = v[1..].trim().to_string();
        if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
            val = val[1..val.len() - 1].to_string();
        }
        (key, if val.is_empty() { "ON".into() } else { val })
    } else {
        (raw.trim().to_string(), "ON".to_string())
    }
}
