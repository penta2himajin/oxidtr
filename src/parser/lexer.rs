/// Lexer for Alloy source files.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    // Keywords
    Sig,
    Abstract,
    Extends,
    One,
    Lone,
    Set,
    Seq,
    Fact,
    Pred,
    Fun,
    Assert,
    All,
    Some_,
    No,
    Not,
    And,
    Or,
    Implies,
    Iff,
    In,
    Check,
    Run,
    Disj,
    Var, // Alloy 6: mutable field
    // Symbols
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Colon,
    Comma,
    Dot,
    Hash,
    Caret,
    Eq,
    NotEq,
    Lt,
    Gt,
    Lte,
    Gte,
    Arrow,
    Pipe,
    Plus,
    Ampersand,
    Minus,
    // Literals
    Ident(String),
    Int(i64),
    // Special
    Eof,
}

pub struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Lexer {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.input.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // skip whitespace
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            // skip line comments (-- or //)
            if self.pos + 1 < self.input.len() {
                let pair = (self.input[self.pos], self.input[self.pos + 1]);
                if pair == (b'-', b'-') || pair == (b'/', b'/') {
                    while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                    continue;
                }
                // skip block comments /* ... */
                if pair == (b'/', b'*') {
                    self.pos += 2;
                    while self.pos + 1 < self.input.len() {
                        if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                    continue;
                }
            }
            break;
        }
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.input.len()
            && (self.input[self.pos].is_ascii_alphanumeric()
                || self.input[self.pos] == b'_'
                || self.input[self.pos] == b'\'')
        {
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.input[start..self.pos]).into_owned()
    }

    fn read_int(&mut self) -> i64 {
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.input[start..self.pos]).unwrap();
        s.parse().unwrap()
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        let Some(b) = self.peek_byte() else {
            return Token::Eof;
        };

        // Symbols
        match b {
            b'{' => { self.advance(); return Token::LBrace; }
            b'}' => { self.advance(); return Token::RBrace; }
            b'[' => { self.advance(); return Token::LBracket; }
            b']' => { self.advance(); return Token::RBracket; }
            b'(' => { self.advance(); return Token::LParen; }
            b')' => { self.advance(); return Token::RParen; }
            b':' => { self.advance(); return Token::Colon; }
            b',' => { self.advance(); return Token::Comma; }
            b'.' => { self.advance(); return Token::Dot; }
            b'#' => { self.advance(); return Token::Hash; }
            b'^' => { self.advance(); return Token::Caret; }
            b'|' => { self.advance(); return Token::Pipe; }
            b'<' => {
                self.advance();
                if self.peek_byte() == Some(b'=') {
                    self.advance();
                    return Token::Lte;
                }
                return Token::Lt;
            }
            b'>' => {
                self.advance();
                if self.peek_byte() == Some(b'=') {
                    self.advance();
                    return Token::Gte;
                }
                return Token::Gt;
            }
            b'=' => {
                self.advance();
                if self.peek_byte() == Some(b'>') {
                    self.advance();
                    return Token::Implies;
                }
                return Token::Eq;
            }
            b'!' => {
                self.advance();
                if self.peek_byte() == Some(b'=') {
                    self.advance();
                    return Token::NotEq;
                }
                return Token::Not;
            }
            b'+' => { self.advance(); return Token::Plus; }
            b'&' => { self.advance(); return Token::Ampersand; }
            b'-' => {
                self.advance();
                if self.peek_byte() == Some(b'>') {
                    self.advance();
                    return Token::Arrow;
                }
                // negative int
                if self.peek_byte().map_or(false, |b| b.is_ascii_digit()) {
                    let n = self.read_int();
                    return Token::Int(-n);
                }
                return Token::Minus;
            }
            _ => {}
        }

        // Numbers
        if b.is_ascii_digit() {
            return Token::Int(self.read_int());
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            let ident = self.read_ident();
            return match ident.as_str() {
                "sig" => Token::Sig,
                "abstract" => Token::Abstract,
                "extends" => Token::Extends,
                "one" => Token::One,
                "lone" => Token::Lone,
                "set" => Token::Set,
                "seq" => Token::Seq,
                "fact" => Token::Fact,
                "pred" => Token::Pred,
                "fun" => Token::Fun,
                "assert" => Token::Assert,
                "all" => Token::All,
                "some" => Token::Some_,
                "no" => Token::No,
                "not" => Token::Not,
                "and" => Token::And,
                "or" => Token::Or,
                "implies" => Token::Implies,
                "iff" => Token::Iff,
                "in" => Token::In,
                "check" => Token::Check,
                "run" => Token::Run,
                "disj" => Token::Disj,
                "var" => Token::Var,
                _ => Token::Ident(ident),
            };
        }

        // Unknown: skip
        self.advance();
        self.next_token()
    }

    pub fn peek(&mut self) -> Token {
        let saved = self.pos;
        let tok = self.next_token();
        self.pos = saved;
        tok
    }
}
