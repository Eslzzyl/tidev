pub(super) fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();
    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || first_line.starts_with("Error:")
        || first_line.starts_with("failed to read")
        || first_line.contains(" was denied")
        || first_line.contains("Cannot read binary file")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

pub(super) fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denial_results_are_errors_even_without_prefix() {
        assert!(tool_output_is_error("Path '/tmp/file' was denied."));
        assert!(tool_output_is_error(
            "Sensitive file '/tmp/.env' was denied. Reason: protected"
        ));
    }
}
