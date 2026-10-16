use crate::error::LexError;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── 字面量 ──
    Int(i64),
    Float(f64),
    Str(String),
    Rune(char),
    Color(u32),

    // ── 标识符 ──
    Ident(String),

    // ── 关键字 ──
    Fn, Struct, Trait, Impl, If, Else, While, For, In,
    Return, Break, Continue, Match,
    Import, Use, Pub, Const, Extern, As,
    And, Or, Not, True, False,
    Nothing, App, Par, Async, Await, Spawn, It,

    // ── 类型关键字 ──
    TInt, TI8, TI16, TI32, TI64,
    TU1, TU2, TU4, TU8, TU16, TU32, TU64,
    TF32, TF64, TBool, TStr, TRune,

    // ── 运算符 ──
    Plus, Minus, Star, Slash, Percent,
    Eq, Ne, Lt, Gt, Le, Ge,
    Assign, PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    ColonAssign,
    Question, Bang, Pipe, Bar,
    DotDot, DotDotEq,
    Arrow, FatArrow,

    // ── 分隔符 ──
    LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    Comma, Colon, Semicolon, Dot, At,

    // ── 特殊 ──
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    // ── 基础工具 ──

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    // ── 跳过空白与注释 ──

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                None => return Err(LexError::new(self.line, self.col, "未终止的块注释")),
                Some('*') if self.peek_at(1) == Some('/') => {
                    self.advance();
                    self.advance();
                    return Ok(());
                }
                _ => { self.advance(); }
            }
        }
    }

    // ── 读字面量 ──

    fn read_string(&mut self) -> Result<String, LexError> {
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(LexError::new(self.line, self.col, "未终止的字符串")),
                Some('"') => {
                    self.advance();
                    return Ok(s);
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n')  => s.push('\n'),
                        Some('t')  => s.push('\t'),
                        Some('r')  => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('"')  => s.push('"'),
                        // 转义的花括号用哨兵字符标记，避免被当成插值分隔符
                        Some('{')  => s.push('\u{1}'),
                        Some('}')  => s.push('\u{2}'),
                        Some(c) => return Err(LexError::new(
                            self.line, self.col,
                            format!("未知的转义序列 \\{}", c),
                        )),
                        None => return Err(LexError::new(self.line, self.col, "未终止的转义序列")),
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
    }

    fn read_rune(&mut self) -> Result<char, LexError> {
        let c = match self.advance() {
            None => return Err(LexError::new(self.line, self.col, "未终止的符文")),
            Some('\\') => match self.advance() {
                Some('n')  => '\n',
                Some('t')  => '\t',
                Some('r')  => '\r',
                Some('\\') => '\\',
                Some('\'') => '\'',
                Some(c) => return Err(LexError::new(
                    self.line, self.col,
                    format!("未知的转义序列 \\{}", c),
                )),
                None => return Err(LexError::new(self.line, self.col, "未终止的转义序列")),
            },
            Some(c) => c,
        };
        match self.advance() {
            Some('\'') => Ok(c),
            _ => Err(LexError::new(self.line, self.col, "符文必须是单个字符")),
        }
    }

    fn read_number(&mut self) -> Result<TokenKind, LexError> {
        // 十六进制
        if self.peek() == Some('0') && self.peek_at(1) == Some('x') {
            self.advance();
            self.advance();
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    hex.push(c);
                    self.advance();
                } else if c == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
            if hex.is_empty() {
                return Err(LexError::new(self.line, self.col, "无效的十六进制字面量"));
            }
            let n = i64::from_str_radix(&hex, 16)
                .map_err(|_| LexError::new(self.line, self.col, "十六进制字面量溢出"))?;
            return Ok(TokenKind::Int(n));
        }

        // 二进制
        if self.peek() == Some('0') && self.peek_at(1) == Some('b') {
            self.advance();
            self.advance();
            let mut bin = String::new();
            while let Some(c) = self.peek() {
                if c == '0' || c == '1' {
                    bin.push(c);
                    self.advance();
                } else if c == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
            if bin.is_empty() {
                return Err(LexError::new(self.line, self.col, "无效的二进制字面量"));
            }
            let n = i64::from_str_radix(&bin, 2)
                .map_err(|_| LexError::new(self.line, self.col, "二进制字面量溢出"))?;
            return Ok(TokenKind::Int(n));
        }

        // 十进制（可带小数点）
        let mut s = String::new();
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else if c == '_' {
                self.advance();
            } else if c == '.' && !is_float {
                // 区分 `..`（范围）和 `.`（小数点）
                if self.peek_at(1) == Some('.') {
                    break;
                }
                if let Some(next) = self.peek_at(1) {
                    if next.is_ascii_digit() {
                        is_float = true;
                        s.push('.');
                        self.advance();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if is_float {
            let f: f64 = s.parse()
                .map_err(|_| LexError::new(self.line, self.col, "无效的浮点字面量"))?;
            Ok(TokenKind::Float(f))
        } else {
            let n: i64 = s.parse()
                .map_err(|_| LexError::new(self.line, self.col, "整数字面量溢出"))?;
            Ok(TokenKind::Int(n))
        }
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    fn read_color(&mut self) -> Result<u32, LexError> {
        let mut hex = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_hexdigit() {
                hex.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if hex.len() != 6 {
            return Err(LexError::new(
                self.line, self.col,
                "颜色必须是 6 位十六进制（#RRGGBB）",
            ));
        }
        u32::from_str_radix(&hex, 16)
            .map_err(|_| LexError::new(self.line, self.col, "无效的颜色字面量"))
    }

    // ── 主循环 ──

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace();

            let line = self.line;
            let col = self.col;

            let c = match self.peek() {
                None => {
                    tokens.push(Token { kind: TokenKind::Eof, line, col });
                    break;
                }
                Some(c) => c,
            };

            // 换行
            if c == '\n' {
                self.advance();
                tokens.push(Token { kind: TokenKind::Newline, line, col });
                continue;
            }

            // 注释
            if c == '/' {
                match self.peek_at(1) {
                    Some('/') => {
                        self.advance();
                        self.advance();
                        self.skip_line_comment();
                        continue;
                    }
                    Some('*') => {
                        self.advance();
                        self.advance();
                        self.skip_block_comment()?;
                        continue;
                    }
                    _ => {}
                }
            }

            // 颜色 #RRGGBB
            // shebang 或属性 #!... 或 #[...]
            if c == '#' && (self.peek_at(1) == Some('!') || self.peek_at(1) == Some('[')) {
                // 跳到行尾
                while let Some(ch) = self.peek() {
                    if ch == '\n' { break; }
                    self.advance();
                }
                continue;
            }
            if c == '#' {
                self.advance();
                let color = self.read_color()?;
                tokens.push(Token { kind: TokenKind::Color(color), line, col });
                continue;
            }

            // 字符串
            if c == '"' {
                self.advance();
                let s = self.read_string()?;
                tokens.push(Token { kind: TokenKind::Str(s), line, col });
                continue;
            }

            // 符文
            if c == '\'' {
                self.advance();
                let r = self.read_rune()?;
                tokens.push(Token { kind: TokenKind::Rune(r), line, col });
                continue;
            }

            // 数字
            if c.is_ascii_digit() {
                let kind = self.read_number()?;
                tokens.push(Token { kind, line, col });
                continue;
            }

            // 标识符 / 关键字
            if c.is_alphabetic() || c == '_' {
                let s = self.read_ident();
                let kind = keyword(&s).unwrap_or(TokenKind::Ident(s));
                tokens.push(Token { kind, line, col });
                continue;
            }

            // 运算符 / 分隔符
            self.advance();
            let kind = match c {
                '+' => if self.peek() == Some('=') { self.advance(); TokenKind::PlusEq } else { TokenKind::Plus },
                '-' => match self.peek() {
                    Some('=') => { self.advance(); TokenKind::MinusEq }
                    Some('>') => { self.advance(); TokenKind::Arrow }
                    _ => TokenKind::Minus,
                },
                '*' => if self.peek() == Some('=') { self.advance(); TokenKind::StarEq } else { TokenKind::Star },
                '/' => if self.peek() == Some('=') { self.advance(); TokenKind::SlashEq } else { TokenKind::Slash },
                '%' => if self.peek() == Some('=') { self.advance(); TokenKind::PercentEq } else { TokenKind::Percent },
                '=' => match self.peek() {
                    Some('=') => { self.advance(); TokenKind::Eq }
                    Some('>') => { self.advance(); TokenKind::FatArrow }
                    _ => TokenKind::Assign,
                },
                '!' => if self.peek() == Some('=') { self.advance(); TokenKind::Ne } else { TokenKind::Bang },
                '<' => if self.peek() == Some('=') { self.advance(); TokenKind::Le } else { TokenKind::Lt },
                '>' => if self.peek() == Some('=') { self.advance(); TokenKind::Ge } else { TokenKind::Gt },
                ':' => if self.peek() == Some('=') { self.advance(); TokenKind::ColonAssign } else { TokenKind::Colon },
                '?' => TokenKind::Question,
                '|' => if self.peek() == Some('>') { self.advance(); TokenKind::Pipe } else { TokenKind::Bar },
                '.' => {
                    if self.peek() == Some('.') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            TokenKind::DotDotEq
                        } else {
                            TokenKind::DotDot
                        }
                    } else {
                        TokenKind::Dot
                    }
                }
                '(' => TokenKind::LParen,
                ')' => TokenKind::RParen,
                '[' => TokenKind::LBracket,
                ']' => TokenKind::RBracket,
                '{' => TokenKind::LBrace,
                '}' => TokenKind::RBrace,
                ',' => TokenKind::Comma,
                ';' => TokenKind::Semicolon,
                '@' => TokenKind::At,
                _ => return Err(LexError::new(line, col, format!("无效的字符 '{}'", c))),
            };
            tokens.push(Token { kind, line, col });
        }

        Ok(tokens)
    }
}

// ── 关键字表 ──

fn keyword(s: &str) -> Option<TokenKind> {
    Some(match s {
        "fn" => TokenKind::Fn,
        "struct" => TokenKind::Struct,
        "trait" => TokenKind::Trait,
        "impl" => TokenKind::Impl,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "in" => TokenKind::In,
        "return" => TokenKind::Return,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "match" => TokenKind::Match,
        "import" => TokenKind::Import,
        "use" => TokenKind::Use,
        "pub" => TokenKind::Pub,
        "const" => TokenKind::Const,
        "extern" => TokenKind::Extern,
        "as" => TokenKind::As,
        "and" => TokenKind::And,
        "or" => TokenKind::Or,
        "not" => TokenKind::Not,
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "nothing" => TokenKind::Nothing,
        "app" => TokenKind::App,
        "par" => TokenKind::Par,
        "async" => TokenKind::Async,
        "await" => TokenKind::Await,
        "spawn" => TokenKind::Spawn,
        "it" => TokenKind::It,

        // 类型
        "int" => TokenKind::TInt,
        "i8" => TokenKind::TI8,
        "i16" => TokenKind::TI16,
        "i32" => TokenKind::TI32,
        "i64" => TokenKind::TI64,
        "u1" => TokenKind::TU1,
        "u2" => TokenKind::TU2,
        "u4" => TokenKind::TU4,
        "u8" => TokenKind::TU8,
        "u16" => TokenKind::TU16,
        "u32" => TokenKind::TU32,
        "u64" => TokenKind::TU64,
        "f32" => TokenKind::TF32,
        "f64" | "float" => TokenKind::TF64,
        "bool" => TokenKind::TBool,
        "str" => TokenKind::TStr,
        "rune" => TokenKind::TRune,

        _ => return None,
    })
}