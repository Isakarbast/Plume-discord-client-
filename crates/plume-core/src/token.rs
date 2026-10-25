use std::fmt;

/// A Discord account token.
///
/// It is as powerful as the account password, so this type never prints it:
/// `{:?}` shows `Token(<redacted>)` and there is no `Display`. Call
/// [`Token::expose`] only at the exact point it is sent to Discord.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn new(token: impl Into<String>) -> Self {
        // People often copy it with quotes or a trailing newline.
        let token: String = token.into();
        Token(token.trim().trim_matches('"').to_owned())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_token() {
        let token = Token::new("secret-value");
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
    }

    #[test]
    fn trims_quotes_and_whitespace() {
        assert_eq!(Token::new(" \"abc\"\n").expose(), "abc");
    }
}
