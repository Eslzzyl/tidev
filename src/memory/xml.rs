/// Case-insensitive XML tag parsing utilities, extracted from compress.rs.
///
/// Handles common LLM output quirks: markdown code fences, extra prose,
/// case variation in tag names, and missing optional fields.

/// Clean an LLM response that may contain markdown fences or explanatory
/// prose around the XML block. Returns the inner XML content.
pub fn clean_llm_xml_response(raw: &str) -> String {
    let text = raw.trim().to_string();

    // Strip markdown code fences: ```xml ... ``` or ``` ... ```
    if let (Some(start), Some(end)) = (text.find("```"), text.rfind("```")) {
        if start < end {
            let inner_start = match text[start..].find('\n') {
                Some(nl) => start + nl + 1,
                None => start + 3,
            };
            if inner_start < end {
                return text[inner_start..end].trim().to_string();
            }
        }
    }

    // If no fences, try to find the <observation>...</observation> block
    if let Some(tag_start) = find_tag_boundary_ci(&text, "observation", true) {
        if let Some(tag_end) =
            find_tag_boundary_ci(&text[tag_start..], "observation", false)
        {
            return text[tag_start..tag_start + tag_end].trim().to_string();
        }
    }

    // Return as-is; the case-insensitive parser will attempt further
    text
}

/// Find an opening `<tag>` or closing `</tag>` boundary, case-insensitively.
/// Returns the byte index of the start of the tag (`<` character).
fn find_tag_boundary_ci(xml: &str, tag: &str, opening: bool) -> Option<usize> {
    let xml_lower = xml.to_lowercase();
    let pattern = if opening {
        format!("<{}", tag.to_lowercase())
    } else {
        format!("</{}", tag.to_lowercase())
    };
    xml_lower.find(&pattern)
}

/// Case-insensitive single-tag value extraction.
///
/// Strips markdown fences and explanatory prose first, then finds
/// `<tag>...</tag>` ignoring case. Returns `None` when the tag is
/// absent or its content is empty.
pub fn get_xml_tag_ci(xml: &str, tag: &str) -> Option<String> {
    let xml_lower = xml.to_lowercase();
    let open_tag = format!("<{}>", tag.to_lowercase());
    let close_tag = format!("</{}>", tag.to_lowercase());

    let start = xml_lower.find(&open_tag)?;
    let content_start = start + open_tag.len();
    let end = xml_lower[content_start..].find(&close_tag)?;

    let value = xml[content_start..content_start + end].trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

/// Case-insensitive child-tag list extraction.
///
/// Extracts all `<child>...</child>` inside the first `<parent>...</parent>`,
/// ignoring case on all tag names.
pub fn get_xml_children_ci(xml: &str, parent: &str, child: &str) -> Vec<String> {
    let xml_lower = xml.to_lowercase();
    let parent_open = format!("<{}>", parent.to_lowercase());
    let parent_close = format!("</{}>", parent.to_lowercase());

    let s = match xml_lower.find(&parent_open) {
        Some(pos) => pos,
        None => return vec![],
    };
    let e = match xml_lower[s..].find(&parent_close) {
        Some(pos) => pos,
        None => return vec![],
    };
    let section = &xml[s + parent_open.len()..s + e];
    let section_lower = &xml_lower[s + parent_open.len()..s + e];

    let child_open = format!("<{}>", child.to_lowercase());
    let child_close = format!("</{}>", child.to_lowercase());

    let mut result = Vec::new();
    let mut pos = 0;
    while let Some(cs) = section_lower[pos..].find(&child_open) {
        let content_start = pos + cs + child_open.len();
        if let Some(ce) = section_lower[content_start..].find(&child_close) {
            let value = section[content_start..content_start + ce]
                .trim()
                .to_string();
            if !value.is_empty() {
                result.push(value);
            }
            pos = content_start + ce + child_close.len();
        } else {
            break;
        }
    }

    result
}
