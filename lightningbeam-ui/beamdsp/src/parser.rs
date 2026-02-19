use crate::ast::*;
use crate::error::CompileError;
use crate::token::{Span, Token, TokenKind};
use crate::ui_decl::UiElement;

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<&Token, CompileError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            Ok(self.advance())
        } else {
            Err(CompileError::new(
                format!("Expected {:?}, found {:?}", expected, self.peek()),
                self.span(),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<String, CompileError> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(CompileError::new(
                format!("Expected identifier, found {:?}", self.peek()),
                self.span(),
            )),
        }
    }

    fn expect_string(&mut self) -> Result<String, CompileError> {
        match self.peek().clone() {
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            _ => Err(CompileError::new(
                format!("Expected string literal, found {:?}", self.peek()),
                self.span(),
            )),
        }
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn parse(&mut self) -> Result<Script, CompileError> {
        let mut name = String::new();
        let mut category = CategoryKind::Utility;
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut params = Vec::new();
        let mut state = Vec::new();
        let mut ui = None;
        let mut process = Vec::new();

        while *self.peek() != TokenKind::Eof {
            match self.peek() {
                TokenKind::Name => {
                    self.advance();
                    name = self.expect_string()?;
                }
                TokenKind::Category => {
                    self.advance();
                    category = match self.peek() {
                        TokenKind::Generator => { self.advance(); CategoryKind::Generator }
                        TokenKind::Effect => { self.advance(); CategoryKind::Effect }
                        TokenKind::Utility => { self.advance(); CategoryKind::Utility }
                        _ => {
                            return Err(CompileError::new(
                                "Expected generator, effect, or utility",
                                self.span(),
                            ));
                        }
                    };
                }
                TokenKind::Inputs => {
                    self.advance();
                    inputs = self.parse_port_block()?;
                }
                TokenKind::Outputs => {
                    self.advance();
                    outputs = self.parse_port_block()?;
                }
                TokenKind::Params => {
                    self.advance();
                    params = self.parse_params_block()?;
                }
                TokenKind::State => {
                    self.advance();
                    state = self.parse_state_block()?;
                }
                TokenKind::Ui => {
                    self.advance();
                    ui = Some(self.parse_ui_block()?);
                }
                TokenKind::Process => {
                    self.advance();
                    process = self.parse_block()?;
                }
                _ => {
                    return Err(CompileError::new(
                        format!("Unexpected token {:?} at top level", self.peek()),
                        self.span(),
                    ));
                }
            }
        }

        if name.is_empty() {
            return Err(CompileError::new(
                "Script must have a name declaration",
                Span::new(1, 1),
            ));
        }

        Ok(Script {
            name,
            category,
            inputs,
            outputs,
            params,
            state,
            ui,
            process,
        })
    }

    fn parse_port_block(&mut self) -> Result<Vec<PortDecl>, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut ports = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            let span = self.span();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let signal = match self.peek() {
                TokenKind::Audio => { self.advance(); SignalKind::Audio }
                TokenKind::Cv => { self.advance(); SignalKind::Cv }
                TokenKind::Midi => { self.advance(); SignalKind::Midi }
                _ => {
                    return Err(CompileError::new(
                        "Expected audio, cv, or midi",
                        self.span(),
                    ));
                }
            };
            ports.push(PortDecl { name, signal, span });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(ports)
    }

    fn parse_params_block(&mut self) -> Result<Vec<ParamDecl>, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut params = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            let span = self.span();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let default = self.parse_number()?;
            self.expect(&TokenKind::LBracket)?;
            let min = self.parse_number()?;
            self.expect(&TokenKind::Comma)?;
            let max = self.parse_number()?;
            self.expect(&TokenKind::RBracket)?;
            let unit = self.expect_string()?;
            params.push(ParamDecl {
                name,
                default,
                min,
                max,
                unit,
                span,
            });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(params)
    }

    fn parse_number(&mut self) -> Result<f32, CompileError> {
        let negative = self.eat(&TokenKind::Minus);
        let val = match self.peek() {
            TokenKind::FloatLit(v) => {
                let v = *v;
                self.advance();
                v
            }
            TokenKind::IntLit(v) => {
                let v = *v as f32;
                self.advance();
                v
            }
            _ => {
                return Err(CompileError::new(
                    format!("Expected number, found {:?}", self.peek()),
                    self.span(),
                ));
            }
        };
        Ok(if negative { -val } else { val })
    }

    fn parse_state_block(&mut self) -> Result<Vec<StateDecl>, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut decls = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            let span = self.span();
            let name = self.expect_ident()?;
            self.expect(&TokenKind::Colon)?;
            let ty = self.parse_state_type()?;
            decls.push(StateDecl { name, ty, span });
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(decls)
    }

    fn parse_state_type(&mut self) -> Result<StateType, CompileError> {
        match self.peek() {
            TokenKind::F32 => { self.advance(); Ok(StateType::F32) }
            TokenKind::Int => { self.advance(); Ok(StateType::Int) }
            TokenKind::Bool => { self.advance(); Ok(StateType::Bool) }
            TokenKind::Sample => { self.advance(); Ok(StateType::Sample) }
            TokenKind::LBracket => {
                self.advance();
                let size = match self.peek() {
                    TokenKind::IntLit(n) => {
                        let n = *n as usize;
                        self.advance();
                        n
                    }
                    _ => {
                        return Err(CompileError::new(
                            "Expected integer size for array",
                            self.span(),
                        ));
                    }
                };
                self.expect(&TokenKind::RBracket)?;
                match self.peek() {
                    TokenKind::F32 => { self.advance(); Ok(StateType::ArrayF32(size)) }
                    TokenKind::Int => { self.advance(); Ok(StateType::ArrayInt(size)) }
                    _ => Err(CompileError::new("Expected f32 or int after array size", self.span())),
                }
            }
            _ => Err(CompileError::new(
                format!("Expected type (f32, int, bool, sample, [N]f32, [N]int), found {:?}", self.peek()),
                self.span(),
            )),
        }
    }

    fn parse_ui_block(&mut self) -> Result<Vec<UiElement>, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut elements = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            elements.push(self.parse_ui_element()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(elements)
    }

    fn parse_ui_element(&mut self) -> Result<UiElement, CompileError> {
        match self.peek() {
            TokenKind::Param => {
                self.advance();
                let name = self.expect_ident()?;
                Ok(UiElement::Param(name))
            }
            TokenKind::Sample => {
                self.advance();
                let name = self.expect_ident()?;
                Ok(UiElement::Sample(name))
            }
            TokenKind::Group => {
                self.advance();
                let label = self.expect_string()?;
                let children = self.parse_ui_block()?;
                Ok(UiElement::Group { label, children })
            }
            TokenKind::Canvas => {
                self.advance();
                self.expect(&TokenKind::LBracket)?;
                let width = self.parse_number()?;
                self.expect(&TokenKind::Comma)?;
                let height = self.parse_number()?;
                self.expect(&TokenKind::RBracket)?;
                Ok(UiElement::Canvas { width, height })
            }
            TokenKind::Spacer => {
                self.advance();
                let px = self.parse_number()?;
                Ok(UiElement::Spacer(px))
            }
            _ => Err(CompileError::new(
                format!("Expected UI element (param, sample, group, canvas, spacer), found {:?}", self.peek()),
                self.span(),
            )),
        }
    }

    fn parse_block(&mut self) -> Result<Block, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while *self.peek() != TokenKind::RBrace {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, CompileError> {
        match self.peek() {
            TokenKind::Let => self.parse_let(),
            TokenKind::If => self.parse_if(),
            TokenKind::For => self.parse_for(),
            _ => {
                // Assignment or expression statement
                let span = self.span();
                let expr = self.parse_expr()?;

                if self.eat(&TokenKind::Eq) {
                    // This is an assignment: expr = value
                    let value = self.parse_expr()?;
                    self.eat(&TokenKind::Semicolon);
                    let target = self.expr_to_lvalue(expr, span)?;
                    Ok(Stmt::Assign { target, value, span })
                } else {
                    self.eat(&TokenKind::Semicolon);
                    Ok(Stmt::ExprStmt(expr))
                }
            }
        }
    }

    fn expr_to_lvalue(&self, expr: Expr, span: Span) -> Result<LValue, CompileError> {
        match expr {
            Expr::Ident(name, s) => Ok(LValue::Ident(name, s)),
            Expr::Index(base, idx, s) => {
                if let Expr::Ident(name, _) = *base {
                    Ok(LValue::Index(name, idx, s))
                } else {
                    Err(CompileError::new("Invalid assignment target", span))
                }
            }
            _ => Err(CompileError::new("Invalid assignment target", span)),
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, CompileError> {
        let span = self.span();
        self.advance(); // consume 'let'
        let mutable = self.eat(&TokenKind::Mut);
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Eq)?;
        let init = self.parse_expr()?;
        self.eat(&TokenKind::Semicolon);
        Ok(Stmt::Let {
            name,
            mutable,
            init,
            span,
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, CompileError> {
        let span = self.span();
        self.advance(); // consume 'if'
        let cond = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_block = if self.eat(&TokenKind::Else) {
            if *self.peek() == TokenKind::If {
                // else if -> wrap in a block with single if statement
                Some(vec![self.parse_if()?])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_block,
            else_block,
            span,
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, CompileError> {
        let span = self.span();
        self.advance(); // consume 'for'
        let var = self.expect_ident()?;
        self.expect(&TokenKind::In)?;
        // Expect 0..end
        let zero_span = self.span();
        match self.peek() {
            TokenKind::IntLit(0) => { self.advance(); }
            _ => {
                return Err(CompileError::new(
                    "For loop range must start at 0 (e.g. 0..buffer_size)",
                    zero_span,
                ));
            }
        }
        self.expect(&TokenKind::DotDot)?;
        let end = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            var,
            end,
            body,
            span,
        })
    }

    // Expression parsing with precedence climbing

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_and()?;
        while *self.peek() == TokenKind::PipePipe {
            let span = self.span();
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp(Box::new(left), BinOp::Or, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_equality()?;
        while *self.peek() == TokenKind::AmpAmp {
            let span = self.span();
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::BinOp(Box::new(left), BinOp::And, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::BangEq => BinOp::Ne,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinOp(Box::new(left), op, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::Le,
                TokenKind::GtEq => BinOp::Ge,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinOp(Box::new(left), op, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinOp(Box::new(left), op, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            let span = self.span();
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp(Box::new(left), op, Box::new(right), span);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        match self.peek() {
            TokenKind::Minus => {
                let span = self.span();
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp(UnaryOp::Neg, Box::new(expr), span))
            }
            TokenKind::Bang => {
                let span = self.span();
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp(UnaryOp::Not, Box::new(expr), span))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut expr = self.parse_primary()?;

        // Handle indexing: expr[index]
        while *self.peek() == TokenKind::LBracket {
            let span = self.span();
            self.advance();
            let index = self.parse_expr()?;
            self.expect(&TokenKind::RBracket)?;
            expr = Expr::Index(Box::new(expr), Box::new(index), span);
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        let span = self.span();
        match self.peek().clone() {
            TokenKind::FloatLit(v) => {
                self.advance();
                Ok(Expr::FloatLit(v, span))
            }
            TokenKind::IntLit(v) => {
                self.advance();
                Ok(Expr::IntLit(v, span))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::BoolLit(true, span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::BoolLit(false, span))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
            }
            // Cast: int(expr) or float(expr)
            TokenKind::Int => {
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Cast(CastKind::ToInt, Box::new(expr), span))
            }
            TokenKind::F32 => {
                self.advance();
                self.expect(&TokenKind::LParen)?;
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(Expr::Cast(CastKind::ToFloat, Box::new(expr), span))
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.advance();
                // Check if it's a function call
                if *self.peek() == TokenKind::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    if *self.peek() != TokenKind::RParen {
                        args.push(self.parse_expr()?);
                        while self.eat(&TokenKind::Comma) {
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Call(name, args, span))
                } else {
                    Ok(Expr::Ident(name, span))
                }
            }
            _ => Err(CompileError::new(
                format!("Expected expression, found {:?}", self.peek()),
                span,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_script(source: &str) -> Result<Script, CompileError> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(&tokens);
        parser.parse()
    }

    #[test]
    fn test_minimal_script() {
        let script = parse_script(r#"
            name "Test"
            category utility
            process {}
        "#).unwrap();
        assert_eq!(script.name, "Test");
        assert_eq!(script.category, CategoryKind::Utility);
    }

    #[test]
    fn test_ports_and_params() {
        let script = parse_script(r#"
            name "Gain"
            category effect
            inputs {
                audio_in: audio
                cv_mod: cv
            }
            outputs {
                audio_out: audio
            }
            params {
                gain: 1.0 [0.0, 2.0] ""
            }
            process {}
        "#).unwrap();
        assert_eq!(script.inputs.len(), 2);
        assert_eq!(script.outputs.len(), 1);
        assert_eq!(script.params.len(), 1);
        assert_eq!(script.params[0].name, "gain");
        assert_eq!(script.params[0].default, 1.0);
    }

    #[test]
    fn test_state_with_sample() {
        let script = parse_script(r#"
            name "Sampler"
            category generator
            state {
                clip: sample
                phase: f32
                buffer: [4096]f32
                counter: int
            }
            process {}
        "#).unwrap();
        assert_eq!(script.state.len(), 4);
        assert_eq!(script.state[0].ty, StateType::Sample);
        assert_eq!(script.state[1].ty, StateType::F32);
        assert_eq!(script.state[2].ty, StateType::ArrayF32(4096));
        assert_eq!(script.state[3].ty, StateType::Int);
    }

    #[test]
    fn test_process_with_for_loop() {
        let script = parse_script(r#"
            name "Pass"
            category effect
            inputs { audio_in: audio }
            outputs { audio_out: audio }
            process {
                for i in 0..buffer_size {
                    audio_out[i * 2] = audio_in[i * 2];
                    audio_out[i * 2 + 1] = audio_in[i * 2 + 1];
                }
            }
        "#).unwrap();
        assert_eq!(script.process.len(), 1);
    }

    #[test]
    fn test_expressions() {
        let script = parse_script(r#"
            name "Expr"
            category utility
            process {
                let x = 1.0 + 2.0 * 3.0;
                let y = sin(x) + cos(3.14);
                let z = int(x * 100.0);
            }
        "#).unwrap();
        assert_eq!(script.process.len(), 3);
    }

    #[test]
    fn test_ui_block() {
        let script = parse_script(r#"
            name "UI Test"
            category utility
            params {
                gain: 1.0 [0.0, 2.0] ""
                mix: 0.5 [0.0, 1.0] ""
            }
            state {
                clip: sample
            }
            ui {
                sample clip
                param gain
                group "Advanced" {
                    param mix
                }
            }
            process {}
        "#).unwrap();
        let ui = script.ui.unwrap();
        assert_eq!(ui.len(), 3);
    }
}
