pub mod ast;
pub mod error;
pub mod lexer;
pub mod token;
pub mod ui_decl;
pub mod parser;
pub mod validator;
pub mod opcodes;
pub mod codegen;
pub mod vm;

use error::CompileError;
use lexer::Lexer;
use parser::Parser;

pub use error::ScriptError;
pub use ui_decl::{UiDeclaration, UiElement};
pub use vm::{ScriptVM, SampleSlot, DrawVM, DrawCommand, MouseState};

/// Compiled script metadata — everything needed to create a ScriptNode
pub struct CompiledScript {
    pub vm: ScriptVM,
    pub name: String,
    pub category: ast::CategoryKind,
    pub input_ports: Vec<PortInfo>,
    pub output_ports: Vec<PortInfo>,
    pub parameters: Vec<ParamInfo>,
    pub sample_slots: Vec<String>,
    pub ui_declaration: UiDeclaration,
    pub source: String,
    pub draw_vm: Option<DrawVM>,
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub signal: ast::SignalKind,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: String,
}

/// Compile BeamDSP source code into a ready-to-run script
pub fn compile(source: &str) -> Result<CompiledScript, CompileError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;

    let mut parser = Parser::new(&tokens);
    let script = parser.parse()?;

    let validated = validator::validate(&script)?;

    let (vm, ui_decl, draw_vm) = codegen::compile(&validated)?;

    let input_ports = script
        .inputs
        .iter()
        .map(|p| PortInfo {
            name: p.name.clone(),
            signal: p.signal,
        })
        .collect();

    let output_ports = script
        .outputs
        .iter()
        .map(|p| PortInfo {
            name: p.name.clone(),
            signal: p.signal,
        })
        .collect();

    let parameters = script
        .params
        .iter()
        .map(|p| ParamInfo {
            name: p.name.clone(),
            min: p.min,
            max: p.max,
            default: p.default,
            unit: p.unit.clone(),
        })
        .collect();

    let sample_slots = script
        .state
        .iter()
        .filter(|s| s.ty == ast::StateType::Sample)
        .map(|s| s.name.clone())
        .collect();

    Ok(CompiledScript {
        vm,
        name: script.name.clone(),
        category: script.category,
        input_ports,
        output_ports,
        parameters,
        sample_slots,
        ui_declaration: ui_decl,
        source: source.to_string(),
        draw_vm,
    })
}
