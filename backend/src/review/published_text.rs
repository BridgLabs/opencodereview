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

/// Length of the next line in `text` and of the break after it: a newline, or a `\n` escape as
/// in a JSON or string literal key. A last line has a break of 0.
fn next_line(text: &str) -> (usize, usize) {
    let newline = text.find('\n');
    let escaped = text.find("\\n");
    match (newline, escaped) {
        (Some(n), Some(e)) if e < n => (e, 2),
        (Some(n), _) => (n, 1),
        (None, Some(e)) => (e, 2),
        (None, None) => (text.len(), 0),
    }
}

/// A line of PEM key material: its base64 length and whether it is unmistakably key material (a
/// full-width base64 line or an encrypted key's `Proc-Type`/`DEK-Info` field), or `None` when the
/// line is not one. Blank lines (encrypted keys) and short base64 lines (a key's last line)
/// count only between such lines and an `END` marker, so a one-word line after a truncated key
/// stays.
fn key_line(line: &str) -> Option<(usize, bool)> {
    let line = line.trim_matches(|c: char| c.is_whitespace() || "\"',+".contains(c));
    if line.starts_with("Proc-Type:") || line.starts_with("DEK-Info:") {
        return Some((0, true));
    }
    line.chars()
        .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
        .then_some((line.len(), line.len() >= 16))
}

/// How much of `text` after a `BEGIN ... PRIVATE KEY-----` header is key material, through the
/// `END ... PRIVATE KEY-----` marker when it follows the key lines. `None` when no key follows (a
/// header named in prose).
fn key_block_len(text: &str) -> Option<usize> {
    const END: &str = "PRIVATE KEY-----";
    let (mut pos, mut certain_end, mut base64) = (0, 0, 0);
    loop {
        let (len, break_len) = next_line(&text[pos..]);
        let line = &text[pos..pos + len];
        let marker = line.trim_start_matches(|c: char| c.is_whitespace() || "\"',+".contains(c));
        if marker.starts_with("-----END ") {
            // A malformed END marker still ends the key; its lines are redacted as truncated.
            if let Some(close) = line.find(END).filter(|_| base64 > 0) {
                return Some(pos + close + END.len());
            }
            break;
        }
        let Some((chars, certain)) = key_line(line) else { break };
        base64 += chars;
        if certain {
            certain_end = pos + len;
        }
        if break_len == 0 {
            break;
        }
        pos += len + break_len;
    }
    (base64 >= 40).then_some(certain_end)
}

/// Replaces private key blocks, multi-line or escaped on one line, including a truncated one
/// with no `END` marker. A header with no key after it (one named in prose) stays.
fn redact_private_keys(value: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let header = after
            .find("-----")
            .filter(|end| after[..*end].ends_with("PRIVATE KEY") && !after[..*end].contains('\n'));
        let body = header.map(|end| start + BEGIN.len() + end + 5);
        match body.and_then(|body| key_block_len(&rest[body..]).map(|len| body + len)) {
            Some(end) => {
                out.push_str(&rest[..start]);
                out.push_str(REDACTED);
                rest = &rest[end..];
            }
            None => {
                out.push_str(&rest[..start + BEGIN.len()]);
                rest = &rest[start + BEGIN.len()..];
            }
        }
    }
    out.push_str(rest);
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
        // A URL ends at whitespace or at the punctuation that closes or follows it.
        // Input is HTML-escaped, so `&gt;`/`&lt;` close a URL while the `;` of `&amp;` in a query
        // does not.
        out.push(ch);
        if ch.is_whitespace()
            || "([)],'\"".contains(ch)
            || (ch == ';' && (out.ends_with("&gt;") || out.ends_with("&lt;")))
        {
            word_start = out.len();
        }
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
        // A key escaped on one line, as JSON service-account files and string literals hold it.
        let json = r#""private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\nAb12==\n-----END PRIVATE KEY-----\n","#;
        assert_eq!(redact(json), format!(r#""private_key": "{REDACTED}\n","#));
        let concatenated = "key = \"-----BEGIN PRIVATE KEY-----\\n\" +\n  \"MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7\\n\" +\n  \"-----END PRIVATE KEY-----\\n\";";
        assert_eq!(redact(concatenated), format!("key = \"{REDACTED}\\n\";"));
        // An encrypted key has a blank line after its fields, and a last base64 line can be short.
        let encrypted = format!("-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,0A1B\n\n{body}Ab12==\n-----END RSA PRIVATE KEY-----\ndone");
        assert_eq!(redact(&encrypted), format!("{REDACTED}\ndone"));
        // A malformed END marker still ends the key, and the key lines before it are redacted.
        let malformed = format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{body}-----END OPENSSH PRIVATE KEY----\nok");
        assert_eq!(
            redact(&malformed),
            format!("{REDACTED}\n-----END OPENSSH PRIVATE KEY----\nok")
        );
        // One-word lines after a truncated key are prose, not key lines.
        let words = format!("-----BEGIN PRIVATE KEY-----\n{body}Rotate\n\"Note,\"\nThanks");
        assert_eq!(redact(&words), format!("{REDACTED}\nRotate\n\"Note,\"\nThanks"));
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
        for (after_url, neutralized) in [
            ("[docs](https://host)@org/team", "[docs](https://host)@\u{2060}org/team"),
            ("https://example.com,@octocat", "https://example.com,@\u{2060}octocat"),
            (
                "&lt;https://example.com&gt;@octocat",
                "&lt;https://example.com&gt;@\u{2060}octocat",
            ),
        ] {
            assert_eq!(neutralize(after_url), neutralized);
        }
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
            // `text()` escapes `&` before neutralizing, so queries reach it as `&amp;`.
            "https://www.npmjs.com/search?ranking=popularity&amp;q=@types/node",
        ] {
            assert_eq!(neutralize(kept), kept);
        }
    }
}
