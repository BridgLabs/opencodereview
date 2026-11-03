//! Model text that reaches a GitHub comment: credentials are redacted, and Markdown that would
//! make GitHub fetch a URL (images) or notify someone (mentions) is neutralized.

const REDACTED: &str = "[redacted secret]";

fn alnum(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn upper_alnum(ch: char) -> bool {
    ch.is_ascii_uppercase() || ch.is_ascii_digit()
}

fn alnum_underscore(ch: char) -> bool {
    alnum(ch) || ch == '_'
}

/// Credential prefixes with the body length range and alphabet of a real token, so identifiers
/// that only share a prefix (`ASIA_PACIFIC_REGION`, `npm_package_version`) stay readable.
const TOKEN_SHAPES: &[(&str, usize, usize, fn(char) -> bool)] = &[
    ("github_pat_", 22, 255, alnum_underscore),
    ("ghp_", 36, 36, alnum),
    ("gho_", 36, 36, alnum),
    ("ghu_", 36, 36, alnum),
    ("ghs_", 36, 36, alnum),
    ("ghr_", 36, 36, alnum),
    ("glpat-", 20, 64, token_char),
    ("sk-ant-", 20, 255, token_char),
    ("sk-proj-", 20, 255, token_char),
    ("sk_live_", 16, 255, alnum),
    ("rk_live_", 16, 255, alnum),
    ("xoxb-", 10, 255, token_char),
    ("xoxp-", 10, 255, token_char),
    ("xoxa-", 10, 255, token_char),
    ("xoxr-", 10, 255, token_char),
    ("xoxs-", 10, 255, token_char),
    ("AKIA", 16, 16, upper_alnum),
    ("ASIA", 16, 16, upper_alnum),
    ("AIza", 35, 35, token_char),
    ("npm_", 36, 36, alnum),
];

fn credential(token: &str) -> bool {
    TOKEN_SHAPES.iter().any(|(prefix, min, max, alphabet)| {
        token
            .strip_prefix(prefix)
            .is_some_and(|body| (*min..=*max).contains(&body.len()) && body.chars().all(alphabet))
    })
}

fn token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

/// Replaces credential-shaped tokens, private key blocks and URL passwords.
pub(crate) fn redact(value: &str) -> String {
    let value = redact_private_keys(value);
    let mut out = String::with_capacity(value.len());
    let mut rest = value.as_str();
    while let Some(ch) = rest.chars().next() {
        let at_boundary = out.chars().next_back().is_none_or(|prev| !token_char(prev));
        let token_len = rest.find(|c: char| !token_char(c)).unwrap_or(rest.len());
        if at_boundary && credential(&rest[..token_len]) {
            out.push_str(REDACTED);
            rest = &rest[token_len..];
            continue;
        }
        if rest.starts_with("://") {
            if let Some(end) = url_password_end(&rest[3..]) {
                let userinfo = &rest[3..3 + end];
                let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
                out.push_str("://");
                out.push_str(user);
                out.push(':');
                out.push_str(REDACTED);
                rest = &rest[3 + end..];
                continue;
            }
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// End of `user:password` before the `@` of a URL authority, when it has a password.
fn url_password_end(authority: &str) -> Option<usize> {
    let end = authority
        .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
        .unwrap_or(authority.len());
    let at = authority[..end].rfind('@')?;
    let (_, password) = authority[..at].split_once(':')?;
    (!password.is_empty()).then_some(at)
}

fn redact_private_keys(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("-----BEGIN ") {
        let header_end = rest[start..]
            .find("-----\n")
            .or_else(|| rest[start + 11..].find("-----").map(|i| i + 11));
        let is_key = rest[start..]
            .lines()
            .next()
            .is_some_and(|line| line.contains("PRIVATE KEY"));
        let Some(header_end) = header_end.filter(|_| is_key) else {
            out.push_str(&rest[..start + 11]);
            rest = &rest[start + 11..];
            continue;
        };
        out.push_str(&rest[..start]);
        out.push_str(REDACTED);
        let after = &rest[start + header_end..];
        rest = match after.find("-----END ") {
            Some(end) => {
                let tail = &after[end + 9..];
                tail.find("-----").map_or("", |close| &tail[close + 5..])
            }
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Breaks Markdown images and `@` mentions in already escaped text, so a rendered comment never
/// fetches a model-chosen URL or notifies a user or team.
pub(crate) fn neutralize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut prev: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        let next = chars.peek().copied();
        if ch == '!' && next == Some('[') {
            out.push('\\');
        } else if ch == '@'
            && prev.is_none_or(|p| !p.is_alphanumeric() && p != '.' && p != '_' && p != '-')
            && next.is_some_and(|n| n.is_ascii_alphanumeric())
        {
            // A word joiner keeps the text readable while GitHub no longer sees a mention.
            out.push('\u{2060}');
        }
        prev = Some(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_redacted_and_ordinary_text_stays() {
        let github = format!("ghp_{}", "a1".repeat(18));
        let npm = format!("npm_{}", "Ab3".repeat(12));
        let aws = "AKIAIOSFODNN7EXAMPLE";
        let anthropic = format!("sk-ant-api03-{}", "x".repeat(40));
        let text = format!("Token {github} and {aws}, key={anthropic}, {npm}.");
        let redacted = redact(&text);
        assert_eq!(
            redacted,
            format!("Token {REDACTED} and {REDACTED}, key={REDACTED}, {REDACTED}.")
        );
        // Short or embedded look-alikes are not credentials.
        for kept in [
            "ghp_short",
            "the AKIA prefix",
            "mask_ghp_value",
            "risk-ant-model",
            "use npm_config_cache",
            "ASIA_PACIFIC_SOUTHEAST_REGION",
            "AKIAIOSFODNN7EXAMPLEEXTRA",
            "npm_package_dependencies_typescript",
            "ghp_with_underscores_and_enough_characters_here",
        ] {
            assert_eq!(redact(kept), kept);
        }
    }

    #[test]
    fn url_passwords_and_private_keys_are_redacted() {
        assert_eq!(
            redact("Clone https://deploy:hunter2@example.com/repo.git now"),
            format!("Clone https://deploy:{REDACTED}@example.com/repo.git now")
        );
        assert_eq!(
            redact("https://example.com/a@b and git@github.com:x"),
            "https://example.com/a@b and git@github.com:x"
        );
        let key = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nafter";
        assert_eq!(redact(key), format!("before\n{REDACTED}\nafter"));
        let cert = "-----BEGIN CERTIFICATE-----\nMII\n-----END CERTIFICATE-----";
        assert_eq!(redact(cert), cert);
    }

    #[test]
    fn images_and_mentions_are_neutralized() {
        assert_eq!(
            neutralize("see ![x](https://evil/?d=1)"),
            "see !\\[x](https://evil/?d=1)"
        );
        assert_eq!(
            neutralize("ask @octocat or @org/team"),
            "ask @\u{2060}octocat or @\u{2060}org/team"
        );
        assert_eq!(neutralize("(@Override)"), "(@\u{2060}Override)");
        for kept in ["mail me@example.com", "a @ b", "[link](https://example.com)", "wow!"] {
            assert_eq!(neutralize(kept), kept);
        }
    }
}
