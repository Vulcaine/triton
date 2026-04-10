use crate::models::TritonRoot;

pub fn expand_placeholders(input: &str, root: Option<&TritonRoot>, config: Option<&str>) -> String {
    let mut expanded = String::new();
    let mut remaining = input;

    while let Some(start) = remaining.find("${") {
        expanded.push_str(&remaining[..start]);
        let token_body = &remaining[start + 2..];
        let Some(end) = token_body.find('}') else {
            expanded.push_str(&remaining[start..]);
            remaining = "";
            break;
        };

        let key = &token_body[..end];
        let replacement = match key {
            "CONFIG" | "config" => Some(config.unwrap_or("${CMAKE_BUILD_TYPE}").to_string()),
            "app_name" => root.map(|r| r.app_name.clone()),
            "generator" => root.map(|r| r.generator.clone()),
            _ => std::env::var(key).ok(),
        };

        if let Some(value) = replacement {
            expanded.push_str(&value);
        } else {
            expanded.push_str("${");
            expanded.push_str(key);
            expanded.push('}');
        }

        remaining = &token_body[end + 1..];
    }

    expanded.push_str(remaining);
    expanded
}
