use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceWarning {
    pub reason: String,
    pub path: Option<String>,
    pub reference: Option<String>,
}

pub(crate) fn merge(warnings: impl IntoIterator<Item = EvidenceWarning>) -> Vec<EvidenceWarning> {
    warnings
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn markdown(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            escaped.extend(ch.escape_default());
        } else {
            match ch {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '!' | '|' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }
    }
    escaped
}

pub(crate) fn append(body: &str, warnings: &[EvidenceWarning]) -> String {
    if warnings.is_empty() {
        return body.to_string();
    }
    let mut result = body.to_string();
    result.push_str(
        "\n\n> [!WARNING]\n> Review completed with limited evidence. Findings are based on the available evidence.\n>",
    );
    for warning in warnings.iter().take(5) {
        let affected = warning
            .path
            .as_deref()
            .map(markdown)
            .unwrap_or_else(|| "Review scope".into());
        let reason = match warning.reason.as_str() {
            "file_unavailable" => "reported unavailable supporting evidence",
            "evidence_excluded" => "reported excluded supporting evidence",
            "insufficient_evidence" => "reported insufficient supporting evidence",
            _ => "reported an evidence limitation",
        };
        result.push_str(&format!("\n> - {affected}: {reason}. "));
        match &warning.reference {
            Some(reference) => result.push_str(&markdown(reference)),
            None => result.push_str("The missing supporting reference was not specified."),
        }
    }
    if warnings.len() > 5 {
        result.push_str(&format!(
            "\n> - {} additional evidence limitation(s) are recorded in the review audit.",
            warnings.len() - 5
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_are_deduplicated_and_cannot_inject_markdown() {
        let warning = EvidenceWarning {
            reason: "insufficient_evidence".into(),
            path: Some("[link](https://invalid.test)\n> injected".into()),
            reference: Some("<script>untrusted</script>".into()),
        };
        let merged = merge([warning.clone(), warning]);
        assert_eq!(merged.len(), 1);
        let rendered = append("Result", &merged);
        assert!(rendered.contains("\\[link\\]\\("));
        assert!(!rendered.contains("\n> injected"));
        assert!(!rendered.contains("<script>"));
        assert_eq!(append("Result", &[]), "Result");
    }
}
