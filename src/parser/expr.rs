use super::Parser;
use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::{Lexer, TokenKind};

impl Parser {
    pub(crate) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.parse_binary(0)?;

        // 范围 0..10 或 0..=10
        if self.eat(&TokenKind::DotDot) {
            let end = self.parse_binary(0)?;
            return Ok(Expr::Range(Box::new(start), Box::new(end), false));
        }
        if self.eat(&TokenKind::DotDotEq) {
            let end = self.parse_binary(0)?;
            return Ok(Expr::Range(Box::new(start), Box::new(end), true));
        }

        Ok(start)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;

        loop {
            let op = match self.peek_binop() {
                Some((op, prec)) if prec >= min_prec => (op, prec),
                _ => break,
            };

            self.advance();
            let right = self.parse_binary(op.1 + 1)?;
            left = Expr::Binary(Box::new(left), op.0, Box::new(right));
        }

        Ok(left)
    }

    fn peek_binop(&self) -> Option<(BinOp, u8)> {
        Some(match self.peek() {
            TokenKind::Or => (BinOp::Or, 1),
            TokenKind::And => (BinOp::And, 2),
            TokenKind::Eq => (BinOp::Eq, 3),
            TokenKind::Ne => (BinOp::Ne, 3),
            TokenKind::Lt => (BinOp::Lt, 3),
            TokenKind::Gt => (BinOp::Gt, 3),
            TokenKind::Le => (BinOp::Le, 3),
            TokenKind::Ge => (BinOp::Ge, 3),
            TokenKind::Pipe => (BinOp::Pipe, 4),
            TokenKind::Plus => (BinOp::Add, 5),
            TokenKind::Minus => (BinOp::Sub, 5),
            TokenKind::Star => (BinOp::Mul, 6),
            TokenKind::Slash => (BinOp::Div, 6),
            TokenKind::Percent => (BinOp::Mod, 6),
            _ => return None,
        })
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            TokenKind::Minus => {
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Neg, Box::new(e)))
            }
            TokenKind::Not => {
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr::Unary(UnOp::Not, Box::new(e)))
            }
            TokenKind::Await => {
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr::Await(Box::new(e)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.advance();
                    let name = match self.advance().clone() {
                        TokenKind::Ident(s) => s,
                        _ => return Err(self.error("期待字段名")),
                    };
                    expr = Expr::Field(Box::new(expr), name);
                }
                TokenKind::LParen => {
                    self.advance();
                    let is_struct_lit = matches!(self.peek(), TokenKind::Ident(_))
                        && matches!(self.peek_at(1), TokenKind::Colon);

                    if is_struct_lit {
                        let mut fields = Vec::new();
                        self.skip_newlines();
                        if !self.check(&TokenKind::RParen) {
                            loop {
                                self.skip_newlines();
                                let field_name = match self.advance().clone() {
                                    TokenKind::Ident(s) => s,
                                    _ => return Err(self.error("期待字段名")),
                                };
                                self.expect(&TokenKind::Colon, "期待 :")?;
                                let value = self.parse_expr()?;
                                fields.push((field_name, value));

                                self.skip_newlines();
                                if !self.eat(&TokenKind::Comma) {
                                    break;
                                }
                            }
                        }
                        self.skip_newlines();
                        self.expect(&TokenKind::RParen, "期待 )")?;

                        if let Expr::Ident(name) = expr {
                            expr = Expr::StructLit(name, fields);
                        } else {
                            return Err(self.error("结构体构造必须是标识符"));
                        }
                    } else {
                        let args = self.parse_call_args()?;
                        expr = Expr::Call(Box::new(expr), args);
                    }
                }
                TokenKind::LBracket => {
                    self.advance();

                    // 空起点切片 [..end] 或 [..]
                    if self.eat(&TokenKind::DotDot) {
                        let end = if self.check(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_binary(0)?))
                        };
                        self.expect(&TokenKind::RBracket, "期待 ]")?;
                        expr = Expr::Slice(Box::new(expr), None, end, false);
                        continue;
                    }

                    let index = self.parse_binary(0)?;

                    if self.eat(&TokenKind::DotDot) {
                        let end = if self.check(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_binary(0)?))
                        };
                        self.expect(&TokenKind::RBracket, "期待 ]")?;
                        expr = Expr::Slice(
                            Box::new(expr),
                            Some(Box::new(index)),
                            end,
                            false,
                        );
                    } else if self.eat(&TokenKind::DotDotEq) {
                        let end = self.parse_binary(0)?;
                        self.expect(&TokenKind::RBracket, "期待 ]")?;
                        expr = Expr::Slice(
                            Box::new(expr),
                            Some(Box::new(index)),
                            Some(Box::new(end)),
                            true,
                        );
                    } else {
                        self.expect(&TokenKind::RBracket, "期待 ]")?;
                        expr = Expr::Index(Box::new(expr), Box::new(index));
                    }
                }
                TokenKind::Question => {
                    self.advance();
                    expr = Expr::Try(Box::new(expr));
                }
                TokenKind::Bang => {
                    self.advance();
                    expr = Expr::Force(Box::new(expr));
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        self.skip_newlines();
        if self.check(&TokenKind::RParen) {
            self.advance();
            return Ok(args);
        }

        loop {
            self.skip_newlines();
            args.push(self.parse_expr()?);
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        self.skip_newlines();
        self.expect(&TokenKind::RParen, "期待 )")?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.advance().clone() {
            TokenKind::Int(n) => Ok(Expr::Int(n)),
            TokenKind::Float(f) => Ok(Expr::Float(f)),
            TokenKind::Str(s) => Ok(self.parse_str_parts(&s)?),
            TokenKind::Rune(c) => Ok(Expr::Rune(c)),
            TokenKind::Color(c) => Ok(Expr::Color(c)),
            TokenKind::True => Ok(Expr::Bool(true)),
            TokenKind::False => Ok(Expr::Bool(false)),
            TokenKind::Nothing => Ok(Expr::Nothing),
            TokenKind::It => Ok(Expr::It),

            TokenKind::Ident(name) => Ok(Expr::Ident(name)),

            TokenKind::Fn => {
                self.expect(&TokenKind::LParen, "期待 (")?;
                let params = self.parse_params()?;
                self.expect(&TokenKind::RParen, "期待 )")?;
                if self.eat(&TokenKind::Arrow) {
                    let _ = self.parse_type()?;
                }
                let body = self.parse_block()?;
                Ok(Expr::Lambda { params, body })
            }

            TokenKind::LParen => {
                let first = self.parse_expr()?;
                if self.eat(&TokenKind::Comma) {
                    let mut items = vec![first];
                    loop {
                        items.push(self.parse_expr()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen, "期待 )")?;
                    Ok(Expr::Tuple(items))
                } else {
                    self.expect(&TokenKind::RParen, "期待 )")?;
                    Ok(first)
                }
            }

            TokenKind::LBracket => {
                let mut items = Vec::new();
                self.skip_newlines();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        items.push(self.parse_expr()?);
                        self.skip_newlines();
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                    }
                }
                self.expect(&TokenKind::RBracket, "期待 ]")?;
                Ok(Expr::List(items))
            }

            TokenKind::If => self.parse_if_expr(),

            _ => Err(self.error("期待表达式")),
        }
    }

    /// if 表达式。支持 `else if` 链：
    /// 内层 if 包成「只有一条表达式语句的块」，这样 els 保持
    /// Option<Block> 不用改类型，single_expr 也能透过它拿到内层 if，
    /// 由 gen_expr 递归展开成嵌套三元。
    /// if 表达式。支持 `else if` 链：
    /// 内层 if 包成「只有一条表达式语句的块」，这样 els 保持
    /// Option<Block> 不用改类型，single_expr 也能透过它拿到内层 if，
    /// 由 gen_expr 递归展开成嵌套三元。
    ///
    /// 注意：本函数假设 `if` 关键字已经被消费掉（parse_primary 的 match
    /// 会先 advance）。所以 else-if 分支里必须自己 advance 吃掉那个 if，
    /// 否则内层的 parse_expr 会把 `if` 当成表达式起点，多套一层。
    fn parse_if_expr(&mut self) -> Result<Expr, ParseError> {
        let cond = self.parse_expr()?;
        let then = self.parse_block()?;
        let els = if self.eat(&TokenKind::Else) {
            if self.check(&TokenKind::If) {
                self.advance();
                let inner = self.parse_if_expr()?;
                Some(Block { stmts: vec![Stmt::Expr(inner)] })
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Expr::If(Box::new(cond), then, els))
    }

    // ── 字符串插值 ──
    // 在解析期把 "a = {x}" 切成 [Text("a = "), Expr(x)]，
    // codegen 之后直接按 AST 推断类型，不再对源码文本做启发式匹配。

    fn parse_str_parts(&mut self, raw: &str) -> Result<Expr, ParseError> {
        let chars: Vec<char> = raw.chars().collect();
        let mut parts: Vec<StrPart> = Vec::new();
        let mut text = String::new();
        let mut i = 0usize;

        while i < chars.len() {
            match chars[i] {
                // 转义花括号的哨兵（lexer 写入）
                '\u{1}' => {
                    text.push('{');
                    i += 1;
                }
                '\u{2}' => {
                    text.push('}');
                    i += 1;
                }
                '{' => {
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        text.push('{');
                        i += 2;
                        continue;
                    }
                    // 找配对的 }，支持嵌套
                    let mut depth = 1;
                    let mut j = i + 1;
                    while j < chars.len() && depth > 0 {
                        if chars[j] == '{' {
                            depth += 1;
                        }
                        if chars[j] == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        j += 1;
                    }
                    if depth != 0 {
                        text.push('{');
                        i += 1;
                        continue;
                    }
                    let inner: String = chars[i + 1..j].iter().collect();
                    if !text.is_empty() {
                        parts.push(StrPart::Text(std::mem::take(&mut text)));
                    }
                    parts.push(StrPart::Expr(self.parse_expr_src(&inner)?));
                    i = j + 1;
                }
                '}' => {
                    if i + 1 < chars.len() && chars[i + 1] == '}' {
                        text.push('}');
                        i += 2;
                        continue;
                    }
                    text.push('}');
                    i += 1;
                }
                c => {
                    text.push(c);
                    i += 1;
                }
            }
        }

        if parts.is_empty() {
            return Ok(Expr::Str(text));
        }
        if !text.is_empty() {
            parts.push(StrPart::Text(text));
        }
        Ok(Expr::FmtStr(parts))
    }

    // 把插值里的源码片段用真正的 lexer + parser 解析成 Expr
    fn parse_expr_src(&self, src: &str) -> Result<Expr, ParseError> {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().map_err(|e| ParseError {
            line: e.line,
            col: e.col,
            msg: e.msg,
        })?;
        let mut sub = Parser::new(tokens);
        sub.parse_expr()
    }
}