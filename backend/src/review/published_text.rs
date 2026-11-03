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

/// A line of PEM key material: base64, or an encrypted key's `Proc-Type`/`DEK-Info` header.
fn key_body(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("Proc-Type:")
        || line.starts_with("DEK-Info:")
        || (line.len() >= 16 && line.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)))
}

/// Replaces a private key block: its `BEGIN ... PRIVATE KEY` line, the key lines that follow and
/// the `END` line when present. A header with no key lines after it (one named in prose) stays.
fn redact_private_keys(value: &str) -> String {
    let lines: Vec<&str> = value.split_inclusive('\n').collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let header = line
            .find("-----BEGIN ")
            .filter(|start| line[*start..].trim_end().ends_with("PRIVATE KEY-----"));
        let body_end = (index + 1..lines.len())
            .find(|next| !key_body(lines[*next]))
            .unwrap_or(lines.len());
        match header {
            Some(start) if body_end > index + 1 => {
                let end_line = lines
                    .get(body_end)
                    .filter(|end| end.trim_start().starts_with("-----END ") && end.contains("PRIVATE KEY-----"));
                let last = end_line.copied().unwrap_or(lines[body_end - 1]);
                out.push_str(&line[..start]);
                out.push_str(REDACTED);
                if last.ends_with('\n') {
                    out.push('\n');
                }
                index = body_end + usize::from(end_line.is_some());
            }
            _ => {
                out.push_str(line);
                index += 1;
            }
        }
    }
    out
}

/// Breaks Markdown images and `@` mentions in already escaped text, so a rendered comment never
/// fetches a model-chosen URL or notifies a user or team.
pub(crate) fn neutralize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut word_start = 0;
    let mut prev: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() || "([".contains(ch) {
            word_start = out.len() + ch.len_utf8();
        }
        out.push(ch);
        let next = chars.peek().copied();
        if ch == '!' && next == Some('[') {
            out.push('\\');
        } else if ch == '@'
            // GitHub creates no mention inside a URL or path (`npmjs.com/package/@scope/pkg`),
            // and a word joiner there would change the link.
            && !out[word_start..].contains("://")
            && prev.is_none_or(|p| !p.is_alphanumeric() && !"._-/".contains(p))
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
        let body = "MIIEpAIBAAKCAQEA1234567890abcdef\nZXhhbXBsZSBrZXkgbWF0ZXJpYWw=\n";
        let key = format!("before\n-----BEGIN RSA PRIVATE KEY-----\n{body}-----END RSA PRIVATE KEY-----\nafter");
        assert_eq!(redact(&key), format!("before\n{REDACTED}\nafter"));
        // A truncated key loses its key lines but keeps the text after them.
        let truncated = format!("Quote:\n-----BEGIN OPENSSH PRIVATE KEY-----\n{body}\nRotate the key.");
        assert_eq!(redact(&truncated), format!("Quote:\n{REDACTED}\n\nRotate the key."));
        // A header named in prose is not a key, and nothing after it is dropped.
        let prose = "config/id_rsa contains a -----BEGIN OPENSSH PRIVATE KEY----- block; remove it and rotate the key";
        assert_eq!(redact(prose), prose);
        let two = "Header -----BEGIN RSA PRIVATE KEY-----\nmentioned, then -----END RSA PRIVATE KEY----- later";
        assert_eq!(redact(two), two);
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
        assert_eq!(
            neutralize("see https://host/@a then @octocat"),
            "see https://host/@a then @\u{2060}octocat"
        );
        for kept in [
            "mail me@example.com",
            "a @ b",
            "[link](https://example.com)",
            "wow!",
            "https://www.npmjs.com/package/@scope/pkg",
            "npmjs.com/package/@scope/pkg",
            "[docs](https://host/@user) and https://mastodon.social/@user",
            "https://host/x?u=@name",
        ] {
            assert_eq!(neutralize(kept), kept);
        }
    }
}
