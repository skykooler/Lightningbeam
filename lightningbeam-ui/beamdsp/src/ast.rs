use crate::token::Span;
use crate::ui_decl::UiElement;

/// Top-level script AST
#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    pub category: CategoryKind,
    pub inputs: Vec<PortDecl>,
    pub outputs: Vec<PortDecl>,
    pub params: Vec<ParamDecl>,
    pub state: Vec<StateDecl>,
    pub ui: Option<Vec<UiElement>>,
    pub process: Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryKind {
    Generator,
    Effect,
    Utility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Audio,
    Cv,
    Midi,
}

#[derive(Debug, Clone)]
pub struct PortDecl {
    pub name: String,
    pub signal: SignalKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub name: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub unit: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StateDecl {
    pub name: String,
    pub ty: StateType,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StateType {
    F32,
    Int,
    Bool,
    ArrayF32(usize),
    ArrayInt(usize),
    Sample,
}

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        init: Expr,
        span: Span,
    },
    Assign {
        target: LValue,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_block: Block,
        else_block: Option<Block>,
        span: Span,
    },
    For {
        var: String,
        end: Expr,
        body: Block,
        span: Span,
    },
    ExprStmt(Expr),
}

#[derive(Debug, Clone)]
pub enum LValue {
    Ident(String, Span),
    Index(String, Box<Expr>, Span),
}

#[derive(Debug, Clone)]
pub enum Expr {
    FloatLit(f32, Span),
    IntLit(i32, Span),
    BoolLit(bool, Span),
    Ident(String, Span),
    BinOp(Box<Expr>, BinOp, Box<Expr>, Span),
    UnaryOp(UnaryOp, Box<Expr>, Span),
    Call(String, Vec<Expr>, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Cast(CastKind, Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::FloatLit(_, s) => *s,
            Expr::IntLit(_, s) => *s,
            Expr::BoolLit(_, s) => *s,
            Expr::Ident(_, s) => *s,
            Expr::BinOp(_, _, _, s) => *s,
            Expr::UnaryOp(_, _, s) => *s,
            Expr::Call(_, _, s) => *s,
            Expr::Index(_, _, s) => *s,
            Expr::Cast(_, _, s) => *s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    ToInt,
    ToFloat,
}
