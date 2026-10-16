mod expr;
mod items;
mod stmt;
mod types;

use crate::error::ParseError;
use crate::lexer::{Token, TokenKind};

pub struct Parser {
    pub(crate) tokens: Vec<Token>,
    pub(crate) pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    // ── 基础工具 ──

    pub(crate) fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    pub(crate) fn peek_at(&self, offset: usize) -> &TokenKind {
        let i = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[i].kind
    }

    pub(crate) fn advance(&mut self) -> &TokenKind {
        let k = &self.tokens[self.pos].kind;
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        k
    }

    pub(crate) fn check(&self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    pub(crate) fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, kind: &TokenKind, msg: &str) -> Result<(), ParseError> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.error(msg))
        }
    }

    pub(crate) fn error(&self, msg: &str) -> ParseError {
        let t = &self.tokens[self.pos];
        ParseError {
            line: t.line,
            col: t.col,
            msg: msg.to_string(),
        }
    }

    pub(crate) fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline | TokenKind::Semicolon) {
            self.advance();
        }
    }

    // ── 入口 ──

    pub fn parse_program(&mut self) -> Result<crate::ast::Program, ParseError> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek(), TokenKind::Eof) {
            items.push(self.parse_item()?);
            self.skip_newlines();
        }
        Ok(crate::ast::Program { items })
    }
}