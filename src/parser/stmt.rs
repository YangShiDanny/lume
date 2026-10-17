use super::Parser;
use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::TokenKind;

impl Parser {
    pub(crate) fn parse_block(&mut self) -> Result<Block, ParseError> {
        self.expect(&TokenKind::LBrace, "期待 {")?;
        self.skip_newlines();

        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }

        self.expect(&TokenKind::RBrace, "期待 }")?;
        Ok(Block { stmts })
    }

    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek() {
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(false),
            TokenKind::Par => {
                self.advance();
                if !self.check(&TokenKind::For) {
                    return Err(self.error("par 后必须跟 for"));
                }
                self.parse_for(true)
            }
            TokenKind::Return => {
                self.advance();
                let value = if matches!(
                    self.peek(),
                    TokenKind::Newline
                        | TokenKind::Semicolon
                        | TokenKind::RBrace
                        | TokenKind::Eof
                ) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Return(value))
            }
            TokenKind::Break => {
                self.advance();
                Ok(Stmt::Break)
            }
            TokenKind::Continue => {
                self.advance();
                Ok(Stmt::Continue)
            }
            TokenKind::Match => self.parse_match_stmt(),
            _ => self.parse_simple_stmt(),
        }
    }

    fn parse_simple_stmt(&mut self) -> Result<Stmt, ParseError> {
        if let TokenKind::Ident(name) = self.peek().clone() {
            // name := expr
            if matches!(self.peek_at(1), TokenKind::ColonAssign) {
                self.advance();
                self.advance();
                let init = self.parse_expr()?;
                return Ok(Stmt::Let {
                    name,
                    ty: None,
                    init,
                });
            }

            // name: Type = expr
            if matches!(self.peek_at(1), TokenKind::Colon) {
                if matches!(
                    self.peek_at(2),
                    TokenKind::TInt
                        | TokenKind::TStr
                        | TokenKind::TBool
                        | TokenKind::TF64
                        | TokenKind::TF32
                        | TokenKind::TRune
                        | TokenKind::TI8
                        | TokenKind::TI16
                        | TokenKind::TI32
                        | TokenKind::TI64
                        | TokenKind::TU1
                        | TokenKind::TU2
                        | TokenKind::TU4
                        | TokenKind::TU8
                        | TokenKind::TU16
                        | TokenKind::TU32
                        | TokenKind::TU64
                | TokenKind::TI128
                | TokenKind::TU128
                        | TokenKind::Ident(_)
                        | TokenKind::LBracket
                ) {
                    self.advance();
                    self.advance();
                    let ty = self.parse_type()?;
                    if self.eat(&TokenKind::Assign) || self.eat(&TokenKind::ColonAssign) {
                        let init = self.parse_expr()?;
                        return Ok(Stmt::Let {
                            name,
                            ty: Some(ty),
                            init,
                        });
                    }
                    return Err(self.error("期待 = 或 :="));
                }
            }
        }

        let expr = self.parse_expr()?;

        if self.eat(&TokenKind::Assign) {
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign {
                target: expr,
                value,
            });
        }

        if matches!(
            self.peek(),
            TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
        ) {
            let op_token = self.advance().clone();
            let value = self.parse_expr()?;
            let bin_op = match op_token {
                TokenKind::PlusEq => BinOp::Add,
                TokenKind::MinusEq => BinOp::Sub,
                TokenKind::StarEq => BinOp::Mul,
                TokenKind::SlashEq => BinOp::Div,
                TokenKind::PercentEq => BinOp::Mod,
                _ => unreachable!(),
            };
            let combined = Expr::Binary(Box::new(expr.clone()), bin_op, Box::new(value));
            return Ok(Stmt::Assign {
                target: expr,
                value: combined,
            });
        }

        Ok(Stmt::Expr(expr))
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::If, "期待 if")?;
        let cond = self.parse_expr()?;
        let then = self.parse_block()?;

        let els = if self.eat(&TokenKind::Else) {
            if self.check(&TokenKind::If) {
                Some(Box::new(self.parse_if()?))
            } else {
                Some(Box::new(Stmt::Expr(Expr::If(
                    Box::new(Expr::Bool(true)),
                    self.parse_block()?,
                    None,
                ))))
            }
        } else {
            None
        };

        Ok(Stmt::If { cond, then, els })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::While, "期待 while")?;
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::While { cond, body })
    }

    fn parse_for(&mut self, is_par: bool) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::For, "期待 for")?;

        let var = match self.advance().clone() {
            TokenKind::Ident(s) => s,
            TokenKind::It => "it".to_string(),
            _ => return Err(self.error("期待循环变量")),
        };

        self.expect(&TokenKind::In, "期待 in")?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;

        if is_par {
            Ok(Stmt::ParFor { var, iter, body })
        } else {
            Ok(Stmt::For { var, iter, body })
        }
    }

    fn parse_match_stmt(&mut self) -> Result<Stmt, ParseError> {
        self.expect(&TokenKind::Match, "期待 match")?;
        let value = self.parse_expr()?;
        self.expect(&TokenKind::LBrace, "期待 {")?;
        self.skip_newlines();

        let mut arms = Vec::new();
        while !self.check(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            let pattern = self.parse_pattern()?;
            self.expect(&TokenKind::FatArrow, "期待 =>")?;
            let body = self.parse_expr()?;
            arms.push(MatchArm { pattern, body });

            self.skip_newlines();
            self.eat(&TokenKind::Comma);
            self.skip_newlines();
        }

        self.expect(&TokenKind::RBrace, "期待 }")?;
        Ok(Stmt::Match { value, arms })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.advance().clone() {
            TokenKind::Ident(s) if s == "_" => Ok(Pattern::Wildcard),
            TokenKind::Ident(s) => {
                if self.eat(&TokenKind::LParen) {
                    let mut subs = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            subs.push(self.parse_pattern()?);
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "期待 )")?;
                    Ok(Pattern::Struct(
                        s,
                        subs.into_iter()
                            .enumerate()
                            .map(|(i, p)| (i.to_string(), p))
                            .collect(),
                    ))
                } else {
                    Ok(Pattern::Ident(s))
                }
            }
            TokenKind::Int(n) => Ok(Pattern::Literal(Expr::Int(n))),
            TokenKind::Str(s) => Ok(Pattern::Literal(Expr::Str(s))),
            TokenKind::True => Ok(Pattern::Literal(Expr::Bool(true))),
            TokenKind::False => Ok(Pattern::Literal(Expr::Bool(false))),
            TokenKind::LParen => {
                let mut subs = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        subs.push(self.parse_pattern()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "期待 )")?;
                Ok(Pattern::Tuple(subs))
            }
            _ => Err(self.error("期待模式")),
        }
    }
}