use super::Parser;
use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::TokenKind;

impl Parser {
    pub(crate) fn parse_type(&mut self) -> Result<Type, ParseError> {
        let base = match self.advance().clone() {
            TokenKind::TInt => Type::Int,
            TokenKind::TI8 => Type::I8,
            TokenKind::TI16 => Type::I16,
            TokenKind::TI32 => Type::I32,
            TokenKind::TI64 => Type::I64,
            TokenKind::TI128 => Type::I128,
            TokenKind::TU1 => Type::U1,
            TokenKind::TU2 => Type::U2,
            TokenKind::TU4 => Type::U4,
            TokenKind::TU8 => Type::U8,
            TokenKind::TU16 => Type::U16,
            TokenKind::TU32 => Type::U32,
            TokenKind::TU64 => Type::U64,
            TokenKind::TU128 => Type::U128,
            TokenKind::TF32 => Type::F32,
            TokenKind::TF64 => Type::F64,
            TokenKind::TBool => Type::Bool,
            TokenKind::TStr => Type::Str,
            TokenKind::TRune => Type::Rune,
            TokenKind::Ident(s) => Type::Named(s),
            TokenKind::LParen => {
                let mut types = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        types.push(self.parse_type()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "期待 )")?;
                if types.len() == 1 {
                    types.into_iter().next().unwrap()
                } else {
                    Type::Tuple(types)
                }
            }
            TokenKind::LBracket => {
                let inner = self.parse_type()?;
                // [T; N] 固定长度数组
                if self.eat(&TokenKind::Semicolon) {
                    let n = match self.advance().clone() {
                        TokenKind::Int(n) => n as usize,
                        _ => return Err(self.error("期待数组长度")),
                    };
                    self.expect(&TokenKind::RBracket, "期待 ]")?;
                    Type::Array(Box::new(inner), Some(n))
                } else {
                    self.expect(&TokenKind::RBracket, "期待 ]")?;
                    Type::List(Box::new(inner))
                }
            }
            _ => return Err(self.error("期待类型")),
        };

        // 后缀：? ! [T]
        let mut ty = base;
        loop {
            match self.peek() {
                TokenKind::Question => {
                    self.advance();
                    ty = Type::Optional(Box::new(ty));
                }
                TokenKind::Bang => {
                    self.advance();
                    ty = Type::Error(Box::new(ty));
                }
                TokenKind::LBracket => {
                    self.advance();
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RBracket) {
                        loop {
                            args.push(self.parse_type()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RBracket, "期待 ]")?;
                    ty = Type::Generic(Box::new(ty), args);
                }
                _ => break,
            }
        }

        Ok(ty)
    }
}