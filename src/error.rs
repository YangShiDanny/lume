use std::fmt;

#[derive(Debug, Clone)]
pub struct LexError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl LexError {
    pub fn new(line: usize, col: usize, msg: impl Into<String>) -> Self {
        Self { line, col, msg: msg.into() }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "error: {}:{}: {}", self.line, self.col, self.msg)
    }
}
#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "error: {}:{}: {}", self.line, self.col, self.msg)
    }
}