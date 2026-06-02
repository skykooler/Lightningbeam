use crate::ast::*;
use crate::error::CompileError;
use crate::token::Span;
use crate::ui_decl::UiElement;

/// Type used during validation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VType {
    F32,
    Int,
    Bool,
    /// Array of f32 (state array or input/output buffer)
    ArrayF32,
    /// Array of int
    ArrayInt,
    /// Sample slot (accessed via sample_read/sample_len)
    Sample,
}

struct VarInfo {
    ty: VType,
    mutable: bool,
}

struct Scope {
    vars: Vec<(String, VarInfo)>,
}

impl Scope {
    fn new() -> Self {
        Self { vars: Vec::new() }
    }

    fn define(&mut self, name: String, ty: VType, mutable: bool) {
        self.vars.push((name, VarInfo { ty, mutable }));
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.vars.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

struct Validator<'a> {
    script: &'a Script,
    scopes: Vec<Scope>,
}

impl<'a> Validator<'a> {
    fn new(script: &'a Script) -> Self {
        Self {
            script,
            scopes: vec![Scope::new()],
        }
    }

    fn current_scope(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.lookup(name) {
                return Some(info);
            }
        }
        None
    }

    fn define(&mut self, name: String, ty: VType, mutable: bool) {
        self.current_scope().define(name, ty, mutable);
    }

    fn validate(&mut self) -> Result<(), CompileError> {
        // Register built-in variables
        self.define("sample_rate".into(), VType::Int, false);
        self.define("buffer_size".into(), VType::Int, false);

        // Register inputs as arrays
        for input in &self.script.inputs {
            let ty = match input.signal {
                SignalKind::Audio | SignalKind::Cv => VType::ArrayF32,
                SignalKind::Midi => continue, // MIDI not yet supported in process
            };
            self.define(input.name.clone(), ty, false);
        }

        // Register outputs as mutable arrays
        for output in &self.script.outputs {
            let ty = match output.signal {
                SignalKind::Audio | SignalKind::Cv => VType::ArrayF32,
                SignalKind::Midi => continue,
            };
            self.define(output.name.clone(), ty, true);
        }

        // Register params as f32
        for param in &self.script.params {
            self.define(param.name.clone(), VType::F32, false);
        }

        // Register state vars
        for state in &self.script.state {
            let (ty, mutable) = match &state.ty {
                StateType::F32 => (VType::F32, true),
                StateType::Int => (VType::Int, true),
                StateType::Bool => (VType::Bool, true),
                StateType::ArrayF32(_) => (VType::ArrayF32, true),
                StateType::ArrayInt(_) => (VType::ArrayInt, true),
                StateType::Sample => (VType::Sample, false),
            };
            self.define(state.name.clone(), ty, mutable);
        }

        // Validate process block
        self.validate_block(&self.script.process)?;

        // Validate UI references
        if let Some(ui) = &self.script.ui {
            self.validate_ui(ui)?;
        }

        Ok(())
    }

    fn validate_block(&mut self, block: &[Stmt]) -> Result<(), CompileError> {
        for stmt in block {
            self.validate_stmt(stmt)?;
        }
        Ok(())
    }

    fn validate_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Let { name, mutable, init, span: _ } => {
                let ty = self.infer_type(init)?;
                self.define(name.clone(), ty, *mutable);
                Ok(())
            }
            Stmt::Assign { target, value, span: _ } => {
                match target {
                    LValue::Ident(name, s) => {
                        let info = self.lookup(name).ok_or_else(|| {
                            CompileError::new(format!("Undefined variable: {}", name), *s)
                        })?;
                        if !info.mutable {
                            return Err(CompileError::new(
                                format!("Cannot assign to immutable variable: {}", name),
                                *s,
                            ));
                        }
                    }
                    LValue::Index(name, idx, s) => {
                        let info = self.lookup(name).ok_or_else(|| {
                            CompileError::new(format!("Undefined variable: {}", name), *s)
                        })?;
                        if !info.mutable {
                            return Err(CompileError::new(
                                format!("Cannot assign to immutable array: {}", name),
                                *s,
                            ));
                        }
                        self.infer_type(idx)?;
                    }
                }
                self.infer_type(value)?;
                Ok(())
            }
            Stmt::If { cond, then_block, else_block, .. } => {
                self.infer_type(cond)?;
                self.push_scope();
                self.validate_block(then_block)?;
                self.pop_scope();
                if let Some(else_b) = else_block {
                    self.push_scope();
                    self.validate_block(else_b)?;
                    self.pop_scope();
                }
                Ok(())
            }
            Stmt::For { var, end, body, span } => {
                let end_ty = self.infer_type(end)?;
                if end_ty != VType::Int {
                    return Err(CompileError::new(
                        "For loop bound must be an integer expression",
                        *span,
                    ).with_hint("Use int(...) to convert, or use buffer_size / len(array)"));
                }
                self.push_scope();
                self.define(var.clone(), VType::Int, false);
                self.validate_block(body)?;
                self.pop_scope();
                Ok(())
            }
            Stmt::ExprStmt(expr) => {
                self.infer_type(expr)?;
                Ok(())
            }
        }
    }

    fn infer_type(&self, expr: &Expr) -> Result<VType, CompileError> {
        match expr {
            Expr::FloatLit(_, _) => Ok(VType::F32),
            Expr::IntLit(_, _) => Ok(VType::Int),
            Expr::BoolLit(_, _) => Ok(VType::Bool),
            Expr::Ident(name, span) => {
                let info = self.lookup(name).ok_or_else(|| {
                    CompileError::new(format!("Undefined variable: {}", name), *span)
                })?;
                Ok(info.ty)
            }
            Expr::BinOp(left, op, right, span) => {
                let lt = self.infer_type(left)?;
                let rt = self.infer_type(right)?;
                match op {
                    BinOp::And | BinOp::Or => Ok(VType::Bool),
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        Ok(VType::Bool)
                    }
                    _ => {
                        // Arithmetic: both sides should be same numeric type
                        if lt == VType::F32 || rt == VType::F32 {
                            Ok(VType::F32)
                        } else if lt == VType::Int && rt == VType::Int {
                            Ok(VType::Int)
                        } else {
                            Err(CompileError::new(
                                format!("Cannot apply {:?} to {:?} and {:?}", op, lt, rt),
                                *span,
                            ))
                        }
                    }
                }
            }
            Expr::UnaryOp(op, inner, _) => {
                let ty = self.infer_type(inner)?;
                match op {
                    UnaryOp::Neg => Ok(ty),
                    UnaryOp::Not => Ok(VType::Bool),
                }
            }
            Expr::Cast(kind, _, _) => match kind {
                CastKind::ToInt => Ok(VType::Int),
                CastKind::ToFloat => Ok(VType::F32),
            },
            Expr::Index(base, idx, span) => {
                let base_ty = self.infer_type(base)?;
                self.infer_type(idx)?;
                match base_ty {
                    VType::ArrayF32 => Ok(VType::F32),
                    VType::ArrayInt => Ok(VType::Int),
                    _ => Err(CompileError::new("Cannot index non-array type", *span)),
                }
            }
            Expr::Call(name, args, span) => {
                self.validate_call(name, args, *span)
            }
        }
    }

    fn validate_call(&self, name: &str, args: &[Expr], span: Span) -> Result<VType, CompileError> {
        // Validate argument count and infer return type
        match name {
            // 1-arg math functions returning f32
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "exp" | "log" | "log2"
            | "sqrt" | "floor" | "ceil" | "round" | "trunc" | "fract" | "abs" | "sign" => {
                if args.len() != 1 {
                    return Err(CompileError::new(format!("{}() takes 1 argument", name), span));
                }
                for arg in args { self.infer_type(arg)?; }
                Ok(VType::F32)
            }
            // 2-arg math functions returning f32
            "atan2" | "pow" | "min" | "max" => {
                if args.len() != 2 {
                    return Err(CompileError::new(format!("{}() takes 2 arguments", name), span));
                }
                for arg in args { self.infer_type(arg)?; }
                Ok(VType::F32)
            }
            // 3-arg functions
            "clamp" | "mix" | "smoothstep" => {
                if args.len() != 3 {
                    return Err(CompileError::new(format!("{}() takes 3 arguments", name), span));
                }
                for arg in args { self.infer_type(arg)?; }
                Ok(VType::F32)
            }
            // cv_or(value, default) -> f32
            "cv_or" => {
                if args.len() != 2 {
                    return Err(CompileError::new("cv_or() takes 2 arguments", span));
                }
                for arg in args { self.infer_type(arg)?; }
                Ok(VType::F32)
            }
            // len(array) -> int
            "len" => {
                if args.len() != 1 {
                    return Err(CompileError::new("len() takes 1 argument", span));
                }
                let ty = self.infer_type(&args[0])?;
                if ty != VType::ArrayF32 && ty != VType::ArrayInt {
                    return Err(CompileError::new("len() requires an array argument", span));
                }
                Ok(VType::Int)
            }
            // sample_len(sample) -> int
            "sample_len" => {
                if args.len() != 1 {
                    return Err(CompileError::new("sample_len() takes 1 argument", span));
                }
                let ty = self.infer_type(&args[0])?;
                if ty != VType::Sample {
                    return Err(CompileError::new("sample_len() requires a sample argument", span));
                }
                Ok(VType::Int)
            }
            // sample_read(sample, index) -> f32
            "sample_read" => {
                if args.len() != 2 {
                    return Err(CompileError::new("sample_read() takes 2 arguments", span));
                }
                let ty = self.infer_type(&args[0])?;
                if ty != VType::Sample {
                    return Err(CompileError::new("sample_read() first argument must be a sample", span));
                }
                self.infer_type(&args[1])?;
                Ok(VType::F32)
            }
            // sample_rate_of(sample) -> int
            "sample_rate_of" => {
                if args.len() != 1 {
                    return Err(CompileError::new("sample_rate_of() takes 1 argument", span));
                }
                let ty = self.infer_type(&args[0])?;
                if ty != VType::Sample {
                    return Err(CompileError::new("sample_rate_of() requires a sample argument", span));
                }
                Ok(VType::Int)
            }
            _ => Err(CompileError::new(format!("Unknown function: {}", name), span)),
        }
    }

    fn validate_ui(&self, elements: &[UiElement]) -> Result<(), CompileError> {
        for element in elements {
            match element {
                UiElement::Param(name) => {
                    if !self.script.params.iter().any(|p| p.name == *name) {
                        return Err(CompileError::new(
                            format!("UI references unknown parameter: {}", name),
                            Span::new(0, 0),
                        ));
                    }
                }
                UiElement::Sample(name) => {
                    if !self.script.state.iter().any(|s| s.name == *name && s.ty == StateType::Sample) {
                        return Err(CompileError::new(
                            format!("UI references unknown sample: {}", name),
                            Span::new(0, 0),
                        ));
                    }
                }
                UiElement::Group { children, .. } => {
                    self.validate_ui(children)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Validate a parsed script. Returns Ok(()) if valid.
pub fn validate(script: &Script) -> Result<&Script, CompileError> {
    let mut validator = Validator::new(script);
    validator.validate()?;
    Ok(script)
}
