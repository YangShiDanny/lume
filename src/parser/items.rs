use super::Parser;
use crate::ast::*;
use crate::error::ParseError;
use crate::lexer::TokenKind;

impl Parser {
    pub(crate) fn parse_item(&mut self) -> Result<Item, ParseError> {
        let decorators = self.parse_decorators()?;
        // 装饰器和 item 之间可能有换行
        self.skip_newlines();

        let is_pub = self.eat(&TokenKind::Pub);

        match self.peek() {
            TokenKind::Fn => self.parse_fn(is_pub, decorators).map(Item::Fn),
            TokenKind::Struct => {
                if !decorators.is_empty() {
                    return Err(self.error("struct 暂不支持装饰器"));
                }
                self.parse_struct(is_pub).map(Item::Struct)
            }
            TokenKind::App => self.parse_app().map(Item::App),
            TokenKind::Const => self.parse_const(is_pub).map(Item::Const),
            TokenKind::Import => self.parse_import().map(Item::Import),
            _ => Err(self.error("期待 fn / struct / app / const / import")),
        }
    }

    pub(crate) fn parse_decorators(&mut self) -> Result<Vec<Decorator>, ParseError> {
        let mut decorators = Vec::new();
        self.skip_newlines();

        while self.eat(&TokenKind::At) {
            let name = match self.advance().clone() {
                TokenKind::Ident(s) => s,
                _ => return Err(self.error("期待装饰器名")),
            };

            let mut args = Vec::new();
            if self.eat(&TokenKind::LParen) {
                if !self.check(&TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "期待 )")?;
            }

            decorators.push(Decorator { name, args });
            // 多个装饰器之间可能有换行
            self.skip_newlines();
        }

        Ok(decorators)
    }

    pub(crate) fn parse_fn(
        &mut self,
        is_pub: bool,
        decorators: Vec<Decorator>,
    ) -> Result<FnDecl, ParseError> {
        self.expect(&TokenKind::Fn, "期待 fn")?;

        let name = match self.advance().clone() {
            TokenKind::Ident(s) => s,
            _ => return Err(self.error("期待函数名")),
        };

        // 泛型参数 [T, U]
        let mut generics = Vec::new();
        if self.eat(&TokenKind::LBracket) {
            loop {
                let name = match self.advance().clone() {
                    TokenKind::Ident(s) => s,
                    _ => return Err(self.error("期待泛型参数名")),
                };
                generics.push(name);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBracket, "期待 ]")?;
        }

        self.expect(&TokenKind::LParen, "期待 (")?;
        let params = self.parse_params()?;
        self.expect(&TokenKind::RParen, "期待 )")?;

        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Ok(FnDecl {
            name,
            generics,
            params,
            ret,
            body,
            is_pub,
            decorators,
        })
    }

    pub(crate) fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        self.skip_newlines();
        if self.check(&TokenKind::RParen) {
            return Ok(params);
        }

        loop {
            self.skip_newlines();
            let name = match self.advance().clone() {
                TokenKind::Ident(s) => s,
                _ => return Err(self.error("期待参数名")),
            };

            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };

            let default = if self.eat(&TokenKind::Assign) {
                Some(self.parse_expr()?)
            } else {
                None
            };

            params.push(Param { name, ty, default });

            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        Ok(params)
    }

    pub(crate) fn parse_struct(&mut self, is_pub: bool) -> Result<StructDecl, ParseError> {
        self.expect(&TokenKind::Struct, "期待 struct")?;

        let name = match self.advance().clone() {
            TokenKind::Ident(s) => s,
            _ => return Err(self.error("期待结构体名")),
        };

        self.expect(&TokenKind::LBrace, "期待 {")?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut methods = Vec::new();

        while !self.check(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            let decorators = self.parse_decorators()?;
            self.skip_newlines();
            let item_pub = self.eat(&TokenKind::Pub);

            if self.check(&TokenKind::Fn) {
                methods.push(self.parse_fn(item_pub, decorators)?);
            } else {
                if !decorators.is_empty() {
                    return Err(self.error("结构体字段不支持装饰器"));
                }
                let fname = match self.advance().clone() {
                    TokenKind::Ident(s) => s,
                    _ => return Err(self.error("期待字段名")),
                };
                self.expect(&TokenKind::Colon, "期待 :")?;
                let fty = self.parse_type()?;
                fields.push(Field {
                    name: fname,
                    ty: fty,
                    is_pub: item_pub,
                });
            }

            self.skip_newlines();
            self.eat(&TokenKind::Comma);
            self.skip_newlines();
        }

        self.expect(&TokenKind::RBrace, "期待 }")?;

        Ok(StructDecl {
            name,
            fields,
            methods,
            is_pub,
        })
    }

    pub(crate) fn parse_app(&mut self) -> Result<AppDecl, ParseError> {
        self.expect(&TokenKind::App, "期待 app")?;

        let name = match self.advance().clone() {
            TokenKind::Ident(s) => s,
            _ => return Err(self.error("期待 app 名")),
        };

        self.expect(&TokenKind::LBrace, "期待 {")?;
        self.skip_newlines();

        let mut body = Vec::new();
        while !self.check(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            body.push(self.parse_stmt()?);
            self.skip_newlines();
        }

        self.expect(&TokenKind::RBrace, "期待 }")?;

        Ok(AppDecl { name, body })
    }

    pub(crate) fn parse_const(&mut self, _is_pub: bool) -> Result<ConstDecl, ParseError> {
        self.expect(&TokenKind::Const, "期待 const")?;

        let name = match self.advance().clone() {
            TokenKind::Ident(s) => s,
            _ => return Err(self.error("期待常量名")),
        };

        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Assign, "期待 =")?;
        let value = self.parse_expr()?;

        Ok(ConstDecl { name, ty, value })
    }

    pub(crate) fn parse_import(&mut self) -> Result<ImportDecl, ParseError> {
        self.expect(&TokenKind::Import, "期待 import")?;

        let mut path = Vec::new();
        loop {
            match self.advance().clone() {
                TokenKind::Ident(s) => path.push(s),
                TokenKind::TStr => path.push("str".to_string()),
                TokenKind::TInt => path.push("int".to_string()),
                TokenKind::TBool => path.push("bool".to_string()),
                TokenKind::TF64 => path.push("float".to_string()),
                _ => return Err(self.error("期待模块名")),
            }
            if !self.eat(&TokenKind::Dot) {
                break;
            }
        }

        let alias = if self.eat(&TokenKind::As) {
            match self.advance().clone() {
                TokenKind::Ident(s) => Some(s),
                _ => return Err(self.error("期待别名")),
            }
        } else {
            None
        };

        Ok(ImportDecl { path, alias })
    }
}