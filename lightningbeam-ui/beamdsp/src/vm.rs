use crate::error::ScriptError;
use crate::opcodes::OpCode;

const STACK_SIZE: usize = 256;
const MAX_LOCALS: usize = 64;
const DEFAULT_INSTRUCTION_LIMIT: u64 = 10_000_000;

/// A value on the VM stack (tagged union)
#[derive(Clone, Copy)]
pub union Value {
    pub f: f32,
    pub i: i32,
    pub b: bool,
}

impl Default for Value {
    fn default() -> Self {
        Value { i: 0 }
    }
}

/// A loaded audio sample slot
#[derive(Clone)]
pub struct SampleSlot {
    pub data: Vec<f32>,
    pub frame_count: usize,
    pub sample_rate: u32,
    pub name: String,
}

impl Default for SampleSlot {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            frame_count: 0,
            sample_rate: 0,
            name: String::new(),
        }
    }
}

/// Result of a single opcode step in VmCore
enum StepResult {
    /// Opcode was handled, continue execution
    Continue,
    /// Hit a Halt instruction
    Halt,
    /// Opcode not handled by core — caller must handle it
    Unhandled(OpCode),
}

/// Shared VM state and opcode dispatch for arithmetic, logic, control flow, and math builtins.
#[derive(Clone)]
struct VmCore {
    bytecode: Vec<u8>,
    constants_f32: Vec<f32>,
    constants_i32: Vec<i32>,
    stack: Vec<Value>,
    sp: usize,
    locals: Vec<Value>,
    params: Vec<f32>,
    state_scalars: Vec<Value>,
    state_arrays: Vec<Vec<f32>>,
    instruction_limit: u64,
}

impl VmCore {
    fn new(
        bytecode: Vec<u8>,
        constants_f32: Vec<f32>,
        constants_i32: Vec<i32>,
        num_params: usize,
        param_defaults: &[f32],
        num_state_scalars: usize,
        state_array_sizes: &[usize],
        instruction_limit: u64,
    ) -> Self {
        let mut params = vec![0.0f32; num_params];
        for (i, &d) in param_defaults.iter().enumerate() {
            if i < params.len() {
                params[i] = d;
            }
        }
        Self {
            bytecode,
            constants_f32,
            constants_i32,
            stack: vec![Value::default(); STACK_SIZE],
            sp: 0,
            locals: vec![Value::default(); MAX_LOCALS],
            params,
            state_scalars: vec![Value::default(); num_state_scalars],
            state_arrays: state_array_sizes.iter().map(|&sz| vec![0.0f32; sz]).collect(),
            instruction_limit,
        }
    }

    /// Reset execution state (sp + locals) at the start of each execute() call.
    fn reset_frame(&mut self) {
        self.sp = 0;
        for l in &mut self.locals {
            *l = Value::default();
        }
    }

    /// Execute one opcode at `pc`. Returns the new `pc` and a `StepResult`.
    /// Handles all opcodes shared between ScriptVM and DrawVM:
    /// stack ops, locals, params, state, arrays, arithmetic, comparison,
    /// logic, casts, control flow, and math builtins.
    fn step(&mut self, pc: &mut usize) -> Result<StepResult, ScriptError> {
        let op = self.bytecode[*pc];
        *pc += 1;

        let Some(opcode) = OpCode::from_u8(op) else {
            return Err(ScriptError::InvalidOpcode(op));
        };

        match opcode {
            OpCode::Halt => return Ok(StepResult::Halt),

            // Stack operations
            OpCode::PushF32 => {
                let idx = self.read_u16(pc) as usize;
                self.push_f(self.constants_f32[idx])?;
            }
            OpCode::PushI32 => {
                let idx = self.read_u16(pc) as usize;
                self.push_i(self.constants_i32[idx])?;
            }
            OpCode::PushBool => {
                let v = self.bytecode[*pc];
                *pc += 1;
                self.push_b(v != 0)?;
            }
            OpCode::Pop => { self.pop()?; }

            // Locals
            OpCode::LoadLocal => {
                let idx = self.read_u16(pc) as usize;
                self.push(self.locals[idx])?;
            }
            OpCode::StoreLocal => {
                let idx = self.read_u16(pc) as usize;
                self.locals[idx] = self.pop()?;
            }

            // Params (read)
            OpCode::LoadParam => {
                let idx = self.read_u16(pc) as usize;
                self.push_f(self.params[idx])?;
            }

            // State scalars
            OpCode::LoadState => {
                let idx = self.read_u16(pc) as usize;
                self.push(self.state_scalars[idx])?;
            }
            OpCode::StoreState => {
                let idx = self.read_u16(pc) as usize;
                self.state_scalars[idx] = self.pop()?;
            }

            // State arrays
            OpCode::LoadStateArray => {
                let arr_id = self.read_u16(pc) as usize;
                let idx = unsafe { self.pop()?.i };
                let val = if arr_id < self.state_arrays.len() {
                    let arr_len = self.state_arrays[arr_id].len();
                    let idx = ((idx % arr_len as i32) + arr_len as i32) as usize % arr_len;
                    self.state_arrays[arr_id][idx]
                } else {
                    0.0
                };
                self.push_f(val)?;
            }
            OpCode::StoreStateArray => {
                let arr_id = self.read_u16(pc) as usize;
                let val = unsafe { self.pop()?.f };
                let idx = unsafe { self.pop()?.i };
                if arr_id < self.state_arrays.len() {
                    let arr_len = self.state_arrays[arr_id].len();
                    let idx = ((idx % arr_len as i32) + arr_len as i32) as usize % arr_len;
                    self.state_arrays[arr_id][idx] = val;
                }
            }
            OpCode::ArrayLen => {
                let arr_id = self.read_u16(pc) as usize;
                let len = if arr_id < self.state_arrays.len() {
                    self.state_arrays[arr_id].len() as i32
                } else {
                    0
                };
                self.push_i(len)?;
            }

            // Float arithmetic
            OpCode::AddF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a + b)?; }
            OpCode::SubF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a - b)?; }
            OpCode::MulF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a * b)?; }
            OpCode::DivF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a / b } else { 0.0 })?; }
            OpCode::ModF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a % b } else { 0.0 })?; }
            OpCode::NegF => { let v = self.pop_f()?; self.push_f(-v)?; }

            // Int arithmetic
            OpCode::AddI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_add(b))?; }
            OpCode::SubI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_sub(b))?; }
            OpCode::MulI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_mul(b))?; }
            OpCode::DivI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a / b } else { 0 })?; }
            OpCode::ModI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a % b } else { 0 })?; }
            OpCode::NegI => { let v = self.pop_i()?; self.push_i(-v)?; }

            // Float comparison
            OpCode::EqF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a == b)?; }
            OpCode::NeF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a != b)?; }
            OpCode::LtF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a < b)?; }
            OpCode::GtF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a > b)?; }
            OpCode::LeF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a <= b)?; }
            OpCode::GeF => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a >= b)?; }

            // Int comparison
            OpCode::EqI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a == b)?; }
            OpCode::NeI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a != b)?; }
            OpCode::LtI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a < b)?; }
            OpCode::GtI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a > b)?; }
            OpCode::LeI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a <= b)?; }
            OpCode::GeI => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a >= b)?; }

            // Logical
            OpCode::And => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a && b)?; }
            OpCode::Or  => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a || b)?; }
            OpCode::Not => { let v = self.pop_b()?; self.push_b(!v)?; }

            // Casts
            OpCode::F32ToI32 => { let v = self.pop_f()?; self.push_i(v as i32)?; }
            OpCode::I32ToF32 => { let v = self.pop_i()?; self.push_f(v as f32)?; }

            // Control flow
            OpCode::Jump => {
                *pc = self.read_u32(pc) as usize;
            }
            OpCode::JumpIfFalse => {
                let target = self.read_u32(pc) as usize;
                let cond = self.pop_b()?;
                if !cond {
                    *pc = target;
                }
            }

            // Math builtins
            OpCode::Sin   => { let v = self.pop_f()?; self.push_f(v.sin())?; }
            OpCode::Cos   => { let v = self.pop_f()?; self.push_f(v.cos())?; }
            OpCode::Tan   => { let v = self.pop_f()?; self.push_f(v.tan())?; }
            OpCode::Asin  => { let v = self.pop_f()?; self.push_f(v.asin())?; }
            OpCode::Acos  => { let v = self.pop_f()?; self.push_f(v.acos())?; }
            OpCode::Atan  => { let v = self.pop_f()?; self.push_f(v.atan())?; }
            OpCode::Atan2 => { let x = self.pop_f()?; let y = self.pop_f()?; self.push_f(y.atan2(x))?; }
            OpCode::Exp   => { let v = self.pop_f()?; self.push_f(v.exp())?; }
            OpCode::Log   => { let v = self.pop_f()?; self.push_f(v.ln())?; }
            OpCode::Log2  => { let v = self.pop_f()?; self.push_f(v.log2())?; }
            OpCode::Pow   => { let e = self.pop_f()?; let b = self.pop_f()?; self.push_f(b.powf(e))?; }
            OpCode::Sqrt  => { let v = self.pop_f()?; self.push_f(v.sqrt())?; }
            OpCode::Floor => { let v = self.pop_f()?; self.push_f(v.floor())?; }
            OpCode::Ceil  => { let v = self.pop_f()?; self.push_f(v.ceil())?; }
            OpCode::Round => { let v = self.pop_f()?; self.push_f(v.round())?; }
            OpCode::Trunc => { let v = self.pop_f()?; self.push_f(v.trunc())?; }
            OpCode::Fract => { let v = self.pop_f()?; self.push_f(v.fract())?; }
            OpCode::Abs   => { let v = self.pop_f()?; self.push_f(v.abs())?; }
            OpCode::Sign  => { let v = self.pop_f()?; self.push_f(v.signum())?; }
            OpCode::Clamp => {
                let hi = self.pop_f()?;
                let lo = self.pop_f()?;
                let v = self.pop_f()?;
                self.push_f(v.clamp(lo, hi))?;
            }
            OpCode::Min => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.min(b))?; }
            OpCode::Max => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.max(b))?; }
            OpCode::Mix => {
                let t = self.pop_f()?;
                let b = self.pop_f()?;
                let a = self.pop_f()?;
                self.push_f(a + (b - a) * t)?;
            }
            OpCode::Smoothstep => {
                let x = self.pop_f()?;
                let e1 = self.pop_f()?;
                let e0 = self.pop_f()?;
                let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                self.push_f(t * t * (3.0 - 2.0 * t))?;
            }
            OpCode::IsNan => { let v = self.pop_f()?; self.push_b(v.is_nan())?; }

            // VM-specific opcodes — caller must handle
            other => return Ok(StepResult::Unhandled(other)),
        }

        Ok(StepResult::Continue)
    }

    // Stack helpers
    #[inline]
    fn push(&mut self, v: Value) -> Result<(), ScriptError> {
        if self.sp >= STACK_SIZE {
            return Err(ScriptError::StackOverflow);
        }
        self.stack[self.sp] = v;
        self.sp += 1;
        Ok(())
    }

    #[inline]
    fn push_f(&mut self, v: f32) -> Result<(), ScriptError> { self.push(Value { f: v }) }
    #[inline]
    fn push_i(&mut self, v: i32) -> Result<(), ScriptError> { self.push(Value { i: v }) }
    #[inline]
    fn push_b(&mut self, v: bool) -> Result<(), ScriptError> { self.push(Value { b: v }) }

    #[inline]
    fn pop(&mut self) -> Result<Value, ScriptError> {
        if self.sp == 0 {
            return Err(ScriptError::StackUnderflow);
        }
        self.sp -= 1;
        Ok(self.stack[self.sp])
    }

    #[inline]
    fn pop_f(&mut self) -> Result<f32, ScriptError> { Ok(unsafe { self.pop()?.f }) }
    #[inline]
    fn pop_i(&mut self) -> Result<i32, ScriptError> { Ok(unsafe { self.pop()?.i }) }
    #[inline]
    fn pop_b(&mut self) -> Result<bool, ScriptError> { Ok(unsafe { self.pop()?.b }) }

    #[inline]
    fn read_u16(&self, pc: &mut usize) -> u16 {
        let v = u16::from_le_bytes([self.bytecode[*pc], self.bytecode[*pc + 1]]);
        *pc += 2;
        v
    }

    #[inline]
    fn read_u32(&self, pc: &mut usize) -> u32 {
        let v = u32::from_le_bytes([
            self.bytecode[*pc], self.bytecode[*pc + 1],
            self.bytecode[*pc + 2], self.bytecode[*pc + 3],
        ]);
        *pc += 4;
        v
    }
}

// ---- ScriptVM (runs on audio thread) ----

/// The BeamDSP virtual machine
#[derive(Clone)]
pub struct ScriptVM {
    core: VmCore,
    pub sample_slots: Vec<SampleSlot>,
}

impl ScriptVM {
    pub fn new(
        bytecode: Vec<u8>,
        constants_f32: Vec<f32>,
        constants_i32: Vec<i32>,
        num_params: usize,
        param_defaults: &[f32],
        num_state_scalars: usize,
        state_array_sizes: &[usize],
        num_sample_slots: usize,
    ) -> Self {
        Self {
            core: VmCore::new(
                bytecode, constants_f32, constants_i32,
                num_params, param_defaults, num_state_scalars, state_array_sizes,
                DEFAULT_INSTRUCTION_LIMIT,
            ),
            sample_slots: (0..num_sample_slots).map(|_| SampleSlot::default()).collect(),
        }
    }

    /// Access params for reading
    pub fn params(&self) -> &[f32] {
        &self.core.params
    }

    /// Access params mutably (backend sets values from parameter changes)
    pub fn params_mut(&mut self) -> &mut Vec<f32> {
        &mut self.core.params
    }

    /// Reset all state (scalars + arrays) to zero. Called on node reset.
    pub fn reset_state(&mut self) {
        for s in &mut self.core.state_scalars {
            *s = Value::default();
        }
        for arr in &mut self.core.state_arrays {
            arr.fill(0.0);
        }
    }

    /// Execute the bytecode with the given I/O buffers
    pub fn execute(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        sample_rate: u32,
        buffer_size: usize,
    ) -> Result<(), ScriptError> {
        self.core.reset_frame();

        let mut pc: usize = 0;
        let mut ic: u64 = 0;
        let limit = self.core.instruction_limit;

        while pc < self.core.bytecode.len() {
            ic += 1;
            if ic > limit {
                return Err(ScriptError::ExecutionLimitExceeded);
            }

            match self.core.step(&mut pc)? {
                StepResult::Continue => {}
                StepResult::Halt => return Ok(()),
                StepResult::Unhandled(opcode) => {
                    match opcode {
                        // Input buffers
                        OpCode::LoadInput => {
                            let port = self.core.bytecode[pc] as usize;
                            pc += 1;
                            let idx = unsafe { self.core.pop()?.i } as usize;
                            let val = if port < inputs.len() && idx < inputs[port].len() {
                                inputs[port][idx]
                            } else {
                                0.0
                            };
                            self.core.push_f(val)?;
                        }

                        // Output buffers
                        OpCode::StoreOutput => {
                            let port = self.core.bytecode[pc] as usize;
                            pc += 1;
                            let val = unsafe { self.core.pop()?.f };
                            let idx = unsafe { self.core.pop()?.i } as usize;
                            if port < outputs.len() && idx < outputs[port].len() {
                                outputs[port][idx] = val;
                            }
                        }

                        // Sample access
                        OpCode::SampleLen => {
                            let slot = self.core.bytecode[pc] as usize;
                            pc += 1;
                            let len = if slot < self.sample_slots.len() {
                                self.sample_slots[slot].frame_count as i32
                            } else {
                                0
                            };
                            self.core.push_i(len)?;
                        }
                        OpCode::SampleRead => {
                            let slot = self.core.bytecode[pc] as usize;
                            pc += 1;
                            let idx = unsafe { self.core.pop()?.i } as usize;
                            let val = if slot < self.sample_slots.len() && idx < self.sample_slots[slot].data.len() {
                                self.sample_slots[slot].data[idx]
                            } else {
                                0.0
                            };
                            self.core.push_f(val)?;
                        }
                        OpCode::SampleRateOf => {
                            let slot = self.core.bytecode[pc] as usize;
                            pc += 1;
                            let sr = if slot < self.sample_slots.len() {
                                self.sample_slots[slot].sample_rate as i32
                            } else {
                                0
                            };
                            self.core.push_i(sr)?;
                        }

                        // Built-in constants
                        OpCode::LoadSampleRate => {
                            self.core.push_i(sample_rate as i32)?;
                        }
                        OpCode::LoadBufferSize => {
                            self.core.push_i(buffer_size as i32)?;
                        }

                        // Draw/mouse opcodes are not valid in the audio ScriptVM
                        _ => {
                            return Err(ScriptError::InvalidOpcode(opcode as u8));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

// ---- Draw VM (runs on UI thread, produces draw commands) ----

/// A draw command produced by the draw block
#[derive(Debug, Clone)]
pub enum DrawCommand {
    FillCircle { cx: f32, cy: f32, r: f32, color: u32 },
    StrokeCircle { cx: f32, cy: f32, r: f32, color: u32, width: f32 },
    StrokeArc { cx: f32, cy: f32, r: f32, start_deg: f32, end_deg: f32, color: u32, width: f32 },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32 },
    FillRect { x: f32, y: f32, w: f32, h: f32, color: u32 },
    StrokeRect { x: f32, y: f32, w: f32, h: f32, color: u32, width: f32 },
}

/// Mouse state passed to the draw VM each frame
#[derive(Debug, Clone, Default)]
pub struct MouseState {
    pub x: f32,
    pub y: f32,
    pub down: bool,
}

/// Lightweight VM for executing draw bytecode on the UI thread
#[derive(Clone)]
pub struct DrawVM {
    core: VmCore,
    pub draw_commands: Vec<DrawCommand>,
    pub mouse: MouseState,
}

impl DrawVM {
    pub fn new(
        bytecode: Vec<u8>,
        constants_f32: Vec<f32>,
        constants_i32: Vec<i32>,
        num_params: usize,
        param_defaults: &[f32],
        num_state_scalars: usize,
        state_array_sizes: &[usize],
    ) -> Self {
        Self {
            core: VmCore::new(
                bytecode, constants_f32, constants_i32,
                num_params, param_defaults, num_state_scalars, state_array_sizes,
                1_000_000, // lower limit for draw (runs per frame)
            ),
            draw_commands: Vec::new(),
            mouse: MouseState::default(),
        }
    }

    /// Access params for reading/writing from the editor
    pub fn params(&self) -> &[f32] {
        &self.core.params
    }

    /// Access params mutably (editor sets values from node inputs each frame)
    pub fn params_mut(&mut self) -> &mut Vec<f32> {
        &mut self.core.params
    }

    /// Check if bytecode is non-empty
    pub fn has_bytecode(&self) -> bool {
        !self.core.bytecode.is_empty()
    }

    /// Execute the draw bytecode. Call once per frame.
    /// Draw commands accumulate in `self.draw_commands` (cleared at start).
    pub fn execute(&mut self) -> Result<(), ScriptError> {
        self.core.reset_frame();
        self.draw_commands.clear();

        let mut pc: usize = 0;
        let mut ic: u64 = 0;
        let limit = self.core.instruction_limit;

        while pc < self.core.bytecode.len() {
            ic += 1;
            if ic > limit {
                return Err(ScriptError::ExecutionLimitExceeded);
            }

            match self.core.step(&mut pc)? {
                StepResult::Continue => {}
                StepResult::Halt => return Ok(()),
                StepResult::Unhandled(opcode) => {
                    match opcode {
                        // Draw commands
                        OpCode::DrawFillCircle => {
                            let color = self.core.pop_i()? as u32;
                            let r = self.core.pop_f()?;
                            let cy = self.core.pop_f()?;
                            let cx = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::FillCircle { cx, cy, r, color });
                        }
                        OpCode::DrawStrokeCircle => {
                            let width = self.core.pop_f()?;
                            let color = self.core.pop_i()? as u32;
                            let r = self.core.pop_f()?;
                            let cy = self.core.pop_f()?;
                            let cx = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::StrokeCircle { cx, cy, r, color, width });
                        }
                        OpCode::DrawStrokeArc => {
                            let width = self.core.pop_f()?;
                            let color = self.core.pop_i()? as u32;
                            let end_deg = self.core.pop_f()?;
                            let start_deg = self.core.pop_f()?;
                            let r = self.core.pop_f()?;
                            let cy = self.core.pop_f()?;
                            let cx = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::StrokeArc { cx, cy, r, start_deg, end_deg, color, width });
                        }
                        OpCode::DrawLine => {
                            let width = self.core.pop_f()?;
                            let color = self.core.pop_i()? as u32;
                            let y2 = self.core.pop_f()?;
                            let x2 = self.core.pop_f()?;
                            let y1 = self.core.pop_f()?;
                            let x1 = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::Line { x1, y1, x2, y2, color, width });
                        }
                        OpCode::DrawFillRect => {
                            let color = self.core.pop_i()? as u32;
                            let h = self.core.pop_f()?;
                            let w = self.core.pop_f()?;
                            let y = self.core.pop_f()?;
                            let x = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::FillRect { x, y, w, h, color });
                        }
                        OpCode::DrawStrokeRect => {
                            let width = self.core.pop_f()?;
                            let color = self.core.pop_i()? as u32;
                            let h = self.core.pop_f()?;
                            let w = self.core.pop_f()?;
                            let y = self.core.pop_f()?;
                            let x = self.core.pop_f()?;
                            self.draw_commands.push(DrawCommand::StrokeRect { x, y, w, h, color, width });
                        }

                        // Mouse input
                        OpCode::MouseX => { self.core.push_f(self.mouse.x)?; }
                        OpCode::MouseY => { self.core.push_f(self.mouse.y)?; }
                        OpCode::MouseDown => { self.core.push_f(if self.mouse.down { 1.0 } else { 0.0 })?; }

                        // Param write
                        OpCode::StoreParam => {
                            let idx = self.core.read_u16(&mut pc) as usize;
                            let val = self.core.pop_f()?;
                            if idx < self.core.params.len() {
                                self.core.params[idx] = val;
                            }
                        }

                        // Sample access not available in draw context
                        OpCode::SampleLen | OpCode::SampleRead | OpCode::SampleRateOf => {
                            pc += 1; // skip slot byte
                            self.core.push_i(0)?;
                        }

                        // Audio I/O not available in draw context
                        _ => {
                            return Err(ScriptError::InvalidOpcode(opcode as u8));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
