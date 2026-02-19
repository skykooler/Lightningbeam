/// Source location
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(line: u32, col: u32) -> Self {
        Self { line, col }
    }
}

/// Token with source location
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Header keywords
    Name,
    Category,
    Inputs,
    Outputs,
    Params,
    State,
    Ui,
    Process,

    // Type keywords
    Audio,
    Cv,
    Midi,
    F32,
    Int,
    Bool,
    Sample,

    // Category values
    Generator,
    Effect,
    Utility,

    // Statement keywords
    Let,
    Mut,
    If,
    Else,
    For,
    In,

    // UI keywords
    Group,
    Param,
    Canvas,
    Spacer,

    // Draw block
    Draw,

    // Literals
    FloatLit(f32),
    IntLit(i32),
    StringLit(String),
    True,
    False,

    // Identifiers
    Ident(String),

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,         // =
    EqEq,       // ==
    BangEq,     // !=
    Lt,         // <
    Gt,         // >
    LtEq,       // <=
    GtEq,       // >=
    AmpAmp,     // &&
    PipePipe,   // ||
    Bang,       // !

    // Delimiters
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    LParen,     // (
    RParen,     // )
    Colon,      // :
    Comma,      // ,
    Semicolon,  // ;
    DotDot,     // ..

    // End of file
    Eof,
}

impl TokenKind {
    /// Try to match an identifier string to a keyword
    pub fn from_ident(s: &str) -> TokenKind {
        match s {
            "name" => TokenKind::Name,
            "category" => TokenKind::Category,
            "inputs" => TokenKind::Inputs,
            "outputs" => TokenKind::Outputs,
            "params" => TokenKind::Params,
            "state" => TokenKind::State,
            "ui" => TokenKind::Ui,
            "process" => TokenKind::Process,
            "audio" => TokenKind::Audio,
            "cv" => TokenKind::Cv,
            "midi" => TokenKind::Midi,
            "f32" => TokenKind::F32,
            "int" => TokenKind::Int,
            "bool" => TokenKind::Bool,
            "sample" => TokenKind::Sample,
            "generator" => TokenKind::Generator,
            "effect" => TokenKind::Effect,
            "utility" => TokenKind::Utility,
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "group" => TokenKind::Group,
            "param" => TokenKind::Param,
            "canvas" => TokenKind::Canvas,
            "spacer" => TokenKind::Spacer,
            "draw" => TokenKind::Draw,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            _ => TokenKind::Ident(s.to_string()),
        }
    }
}
