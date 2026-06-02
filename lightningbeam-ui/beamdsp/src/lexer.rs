use crate::error::CompileError;
use crate::token::{Span, Token, TokenKind};

pub struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, CompileError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.source.len() {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: self.span(),
                });
                break;
            }
            tokens.push(self.next_token()?);
        }
        Ok(tokens)
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.source.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> u8 {
        let ch = self.source[self.pos];
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while self.pos < self.source.len() && self.source[self.pos].is_ascii_whitespace() {
                self.advance();
            }
            // Skip line comments
            if self.pos + 1 < self.source.len()
                && self.source[self.pos] == b'/'
                && self.source[self.pos + 1] == b'/'
            {
                while self.pos < self.source.len() && self.source[self.pos] != b'\n' {
                    self.advance();
                }
                continue;
            }
            break;
        }
    }

    fn next_token(&mut self) -> Result<Token, CompileError> {
        let span = self.span();
        let ch = self.advance();

        match ch {
            b'{' => Ok(Token { kind: TokenKind::LBrace, span }),
            b'}' => Ok(Token { kind: TokenKind::RBrace, span }),
            b'[' => Ok(Token { kind: TokenKind::LBracket, span }),
            b']' => Ok(Token { kind: TokenKind::RBracket, span }),
            b'(' => Ok(Token { kind: TokenKind::LParen, span }),
            b')' => Ok(Token { kind: TokenKind::RParen, span }),
            b':' => Ok(Token { kind: TokenKind::Colon, span }),
            b',' => Ok(Token { kind: TokenKind::Comma, span }),
            b';' => Ok(Token { kind: TokenKind::Semicolon, span }),
            b'+' => Ok(Token { kind: TokenKind::Plus, span }),
            b'-' => Ok(Token { kind: TokenKind::Minus, span }),
            b'*' => Ok(Token { kind: TokenKind::Star, span }),
            b'/' => Ok(Token { kind: TokenKind::Slash, span }),
            b'%' => Ok(Token { kind: TokenKind::Percent, span }),

            b'.' if self.peek() == Some(b'.') => {
                self.advance();
                Ok(Token { kind: TokenKind::DotDot, span })
            }

            b'=' if self.peek() == Some(b'=') => {
                self.advance();
                Ok(Token { kind: TokenKind::EqEq, span })
            }
            b'=' => Ok(Token { kind: TokenKind::Eq, span }),

            b'!' if self.peek() == Some(b'=') => {
                self.advance();
                Ok(Token { kind: TokenKind::BangEq, span })
            }
            b'!' => Ok(Token { kind: TokenKind::Bang, span }),

            b'<' if self.peek() == Some(b'=') => {
                self.advance();
                Ok(Token { kind: TokenKind::LtEq, span })
            }
            b'<' => Ok(Token { kind: TokenKind::Lt, span }),

            b'>' if self.peek() == Some(b'=') => {
                self.advance();
                Ok(Token { kind: TokenKind::GtEq, span })
            }
            b'>' => Ok(Token { kind: TokenKind::Gt, span }),

            b'&' if self.peek() == Some(b'&') => {
                self.advance();
                Ok(Token { kind: TokenKind::AmpAmp, span })
            }

            b'|' if self.peek() == Some(b'|') => {
                self.advance();
                Ok(Token { kind: TokenKind::PipePipe, span })
            }

            b'"' => self.read_string(span),

            ch if ch.is_ascii_digit() => self.read_number(ch, span),

            ch if ch.is_ascii_alphabetic() || ch == b'_' => self.read_ident(ch, span),

            _ => Err(CompileError::new(
                format!("Unexpected character: '{}'", ch as char),
                span,
            )),
        }
    }

    fn read_string(&mut self, span: Span) -> Result<Token, CompileError> {
        let mut s = String::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.advance();
                    return Ok(Token {
                        kind: TokenKind::StringLit(s),
                        span,
                    });
                }
                Some(b'\n') | None => {
                    return Err(CompileError::new("Unterminated string literal", span));
                }
                Some(_) => {
                    s.push(self.advance() as char);
                }
            }
        }
    }

    fn read_number(&mut self, first: u8, span: Span) -> Result<Token, CompileError> {
        // Check for hex literal: 0x...
        if first == b'0' && self.peek() == Some(b'x') {
            self.advance(); // skip 'x'
            let mut hex = String::new();
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    hex.push(self.advance() as char);
                } else {
                    break;
                }
            }
            if hex.is_empty() {
                return Err(CompileError::new("Expected hex digits after 0x", span));
            }
            let val = u32::from_str_radix(&hex, 16)
                .map_err(|_| CompileError::new(format!("Invalid hex literal: 0x{}", hex), span))?;
            return Ok(Token {
                kind: TokenKind::IntLit(val as i32),
                span,
            });
        }

        let mut s = String::new();
        s.push(first as char);
        let mut is_float = false;

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                s.push(self.advance() as char);
            } else if ch == b'.' && self.peek_next() != Some(b'.') && !is_float {
                is_float = true;
                s.push(self.advance() as char);
            } else {
                break;
            }
        }

        if is_float {
            let val: f32 = s
                .parse()
                .map_err(|_| CompileError::new(format!("Invalid float literal: {}", s), span))?;
            Ok(Token {
                kind: TokenKind::FloatLit(val),
                span,
            })
        } else {
            let val: i32 = s
                .parse()
                .map_err(|_| CompileError::new(format!("Invalid integer literal: {}", s), span))?;
            // Check if this could be a float (e.g. 0 used in float context)
            // For now, emit as IntLit; parser/validator handles coercion
            Ok(Token {
                kind: TokenKind::IntLit(val),
                span,
            })
        }
    }

    fn read_ident(&mut self, first: u8, span: Span) -> Result<Token, CompileError> {
        let mut s = String::new();
        s.push(first as char);

        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                s.push(self.advance() as char);
            } else {
                break;
            }
        }

        Ok(Token {
            kind: TokenKind::from_ident(&s),
            span,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_tokens() {
        let mut lexer = Lexer::new("name \"Test\" category effect");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Name);
        assert_eq!(tokens[1].kind, TokenKind::StringLit("Test".into()));
        assert_eq!(tokens[2].kind, TokenKind::Category);
        assert_eq!(tokens[3].kind, TokenKind::Effect);
    }

    #[test]
    fn test_numbers() {
        let mut lexer = Lexer::new("42 3.14 0.5");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLit(42));
        assert_eq!(tokens[1].kind, TokenKind::FloatLit(3.14));
        assert_eq!(tokens[2].kind, TokenKind::FloatLit(0.5));
    }

    #[test]
    fn test_operators() {
        let mut lexer = Lexer::new("== != <= >= && || ..");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::EqEq);
        assert_eq!(tokens[1].kind, TokenKind::BangEq);
        assert_eq!(tokens[2].kind, TokenKind::LtEq);
        assert_eq!(tokens[3].kind, TokenKind::GtEq);
        assert_eq!(tokens[4].kind, TokenKind::AmpAmp);
        assert_eq!(tokens[5].kind, TokenKind::PipePipe);
        assert_eq!(tokens[6].kind, TokenKind::DotDot);
    }

    #[test]
    fn test_comments() {
        let mut lexer = Lexer::new("let x = 5; // comment\nlet y = 10;");
        let tokens = lexer.tokenize().unwrap();
        // Should skip the comment
        assert_eq!(tokens[0].kind, TokenKind::Let);
        assert_eq!(tokens[5].kind, TokenKind::Let);
    }

    #[test]
    fn test_range_vs_float() {
        // "0..10" should parse as IntLit(0), DotDot, IntLit(10), not as a float
        let mut lexer = Lexer::new("0..10");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].kind, TokenKind::IntLit(0));
        assert_eq!(tokens[1].kind, TokenKind::DotDot);
        assert_eq!(tokens[2].kind, TokenKind::IntLit(10));
    }
}
