use super::*;

const URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=fixture&response_type=code&scope=user%3Ainference&code_challenge=abc&state=def";

#[test]
fn setup_output_yields_the_wrapped_url_without_terminal_noise() {
    let wrapped = format!("{}\r\n{}", &URL[..80], &URL[80..]);
    let raw = format!("\u{1b}[2K·Opening browser…\r\nBrowser didn't open? Use the url below to sign in (c to copy)\r\n\r\n\u{1b}[1m{wrapped}\u{1b}[22m\r\n\r\n\r\nPaste code here if prompted >");
    let text = strip_terminal(&raw);
    assert!(!text.contains('\u{1b}'));
    assert_eq!(sign_in_url(&text).as_deref(), Some(URL));
    assert_eq!(sign_in_url("Visit https://example.com/help for details"), None);
}

#[test]
fn setup_output_yields_the_token_without_the_following_text() {
    let token = format!("sk-ant-oat01-{}", "a1B2_c3-".repeat(12));
    let text = strip_terminal(&format!("✓ Long-lived authentication token created successfully!\r\n\r\nYour OAuth token (valid for 1 year):\r\n\r\n{token}\r\nStore this token securely."));
    assert_eq!(oauth_token(&text).as_deref(), Some(token.as_str()));
    assert_eq!(oauth_token("sk-ant-oat01-short"), None);
}

#[test]
fn token_stops_where_the_cli_moves_the_cursor_to_the_next_message() {
    let token = format!("sk-ant-oat01-{}AA", "x".repeat(91));
    for raw in [
        format!("Your OAuth token:\u{1b}[2B{token}\u{1b}[2BStore\u{1b}[1Cthis\u{1b}[1Ctoken"),
        format!("Your OAuth token:\r\n\r\n{token}\r\nStore this token securely."),
        format!("\u{1b}[1m{token}\u{1b}[22m\u{1b}[1A\u{1b}[2KStore"),
    ] {
        assert_eq!(oauth_token(&strip_terminal(&raw)).as_deref(), Some(token.as_str()));
    }
    assert_eq!(
        strip_terminal("Browser\u{1b}[1Cdidn't\u{1b}[1Copen?"),
        "Browser didn't open?"
    );
}

#[test]
fn cli_failures_become_actionable_messages_without_echoing_the_code() {
    let rejected = strip_terminal("secret-code\r\nOAuth error: Invalid code\r\n");
    assert_eq!(
        failure_message(&rejected),
        "Claude did not accept that code. Start again and paste the newest code shown after signing in."
    );
    assert!(!failure_message(&rejected).contains("secret-code"));
    assert_eq!(failure_message(""), "Claude sign-in did not finish. Start again.");
    assert!(super::rejected(&rejected));
    assert!(super::rejected(
        "OAuth error: Invalidcode. Please makesure the fullcde wascopied\nPressEntertoretry."
    ));
    assert!(!super::rejected("This will guide you through long-lived (1-year) auth token setup. Claude subscription required.\nPaste code here if prompted >"));
}
