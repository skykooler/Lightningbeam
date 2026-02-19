use crate::token::Span;
use std::fmt;

/// Compile-time error with source location
#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub span: Span,
    pub hint: Option<String>,
}

impl CompileError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error at line {}, col {}: {}", self.span.line, self.span.col, self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  Hint: {}", hint)?;
        }
        Ok(())
    }
}

/// Runtime error during VM execution
#[derive(Debug, Clone)]
pub enum ScriptError {
    ExecutionLimitExceeded,
    StackOverflow,
    StackUnderflow,
    DivisionByZero,
    IndexOutOfBounds { index: i32, len: usize },
    InvalidOpcode(u8),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::ExecutionLimitExceeded => write!(f, "Execution limit exceeded (possible infinite loop)"),
            ScriptError::StackOverflow => write!(f, "Stack overflow"),
            ScriptError::StackUnderflow => write!(f, "Stack underflow"),
            ScriptError::DivisionByZero => write!(f, "Division by zero"),
            ScriptError::IndexOutOfBounds { index, len } => {
                write!(f, "Index {} out of bounds (length {})", index, len)
            }
            ScriptError::InvalidOpcode(op) => write!(f, "Invalid opcode: {}", op),
        }
    }
}
