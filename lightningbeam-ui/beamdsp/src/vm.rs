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

/// The BeamDSP virtual machine
#[derive(Clone)]
pub struct ScriptVM {
    pub bytecode: Vec<u8>,
    pub constants_f32: Vec<f32>,
    pub constants_i32: Vec<i32>,
    stack: Vec<Value>,
    sp: usize,
    locals: Vec<Value>,
    pub params: Vec<f32>,
    pub state_scalars: Vec<Value>,
    pub state_arrays: Vec<Vec<f32>>,
    pub sample_slots: Vec<SampleSlot>,
    instruction_limit: u64,
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
            sample_slots: (0..num_sample_slots).map(|_| SampleSlot::default()).collect(),
            instruction_limit: DEFAULT_INSTRUCTION_LIMIT,
        }
    }

    /// Reset all state (scalars + arrays) to zero. Called on node reset.
    pub fn reset_state(&mut self) {
        for s in &mut self.state_scalars {
            *s = Value::default();
        }
        for arr in &mut self.state_arrays {
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
        self.sp = 0;
        // Clear locals
        for l in &mut self.locals {
            *l = Value::default();
        }

        let mut pc: usize = 0;
        let mut ic: u64 = 0;
        let limit = self.instruction_limit;

        while pc < self.bytecode.len() {
            ic += 1;
            if ic > limit {
                return Err(ScriptError::ExecutionLimitExceeded);
            }

            let op = self.bytecode[pc];
            pc += 1;

            match OpCode::from_u8(op) {
                Some(OpCode::Halt) => return Ok(()),

                Some(OpCode::PushF32) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let v = self.constants_f32[idx];
                    self.push_f(v)?;
                }
                Some(OpCode::PushI32) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let v = self.constants_i32[idx];
                    self.push_i(v)?;
                }
                Some(OpCode::PushBool) => {
                    let v = self.bytecode[pc];
                    pc += 1;
                    self.push_b(v != 0)?;
                }
                Some(OpCode::Pop) => {
                    self.pop()?;
                }

                // Locals
                Some(OpCode::LoadLocal) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let v = self.locals[idx];
                    self.push(v)?;
                }
                Some(OpCode::StoreLocal) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.locals[idx] = self.pop()?;
                }

                // Params
                Some(OpCode::LoadParam) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let v = self.params[idx];
                    self.push_f(v)?;
                }

                // State scalars
                Some(OpCode::LoadState) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let v = self.state_scalars[idx];
                    self.push(v)?;
                }
                Some(OpCode::StoreState) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.state_scalars[idx] = self.pop()?;
                }

                // Input buffers
                Some(OpCode::LoadInput) => {
                    let port = self.bytecode[pc] as usize;
                    pc += 1;
                    let idx = unsafe { self.pop()?.i } as usize;
                    let val = if port < inputs.len() && idx < inputs[port].len() {
                        inputs[port][idx]
                    } else {
                        0.0
                    };
                    self.push_f(val)?;
                }

                // Output buffers
                Some(OpCode::StoreOutput) => {
                    let port = self.bytecode[pc] as usize;
                    pc += 1;
                    let val = unsafe { self.pop()?.f };
                    let idx = unsafe { self.pop()?.i } as usize;
                    if port < outputs.len() && idx < outputs[port].len() {
                        outputs[port][idx] = val;
                    }
                }

                // State arrays
                Some(OpCode::LoadStateArray) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
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
                Some(OpCode::StoreStateArray) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
                    let val = unsafe { self.pop()?.f };
                    let idx = unsafe { self.pop()?.i };
                    if arr_id < self.state_arrays.len() {
                        let arr_len = self.state_arrays[arr_id].len();
                        let idx = ((idx % arr_len as i32) + arr_len as i32) as usize % arr_len;
                        self.state_arrays[arr_id][idx] = val;
                    }
                }

                // Sample access
                Some(OpCode::SampleLen) => {
                    let slot = self.bytecode[pc] as usize;
                    pc += 1;
                    let len = if slot < self.sample_slots.len() {
                        self.sample_slots[slot].frame_count as i32
                    } else {
                        0
                    };
                    self.push_i(len)?;
                }
                Some(OpCode::SampleRead) => {
                    let slot = self.bytecode[pc] as usize;
                    pc += 1;
                    let idx = unsafe { self.pop()?.i } as usize;
                    let val = if slot < self.sample_slots.len() && idx < self.sample_slots[slot].data.len() {
                        self.sample_slots[slot].data[idx]
                    } else {
                        0.0
                    };
                    self.push_f(val)?;
                }
                Some(OpCode::SampleRateOf) => {
                    let slot = self.bytecode[pc] as usize;
                    pc += 1;
                    let sr = if slot < self.sample_slots.len() {
                        self.sample_slots[slot].sample_rate as i32
                    } else {
                        0
                    };
                    self.push_i(sr)?;
                }

                // Float arithmetic
                Some(OpCode::AddF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a + b)?; }
                Some(OpCode::SubF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a - b)?; }
                Some(OpCode::MulF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a * b)?; }
                Some(OpCode::DivF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a / b } else { 0.0 })?; }
                Some(OpCode::ModF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a % b } else { 0.0 })?; }
                Some(OpCode::NegF) => { let v = self.pop_f()?; self.push_f(-v)?; }

                // Int arithmetic
                Some(OpCode::AddI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_add(b))?; }
                Some(OpCode::SubI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_sub(b))?; }
                Some(OpCode::MulI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_mul(b))?; }
                Some(OpCode::DivI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a / b } else { 0 })?; }
                Some(OpCode::ModI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a % b } else { 0 })?; }
                Some(OpCode::NegI) => { let v = self.pop_i()?; self.push_i(-v)?; }

                // Float comparison
                Some(OpCode::EqF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a == b)?; }
                Some(OpCode::NeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a != b)?; }
                Some(OpCode::LtF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a < b)?; }
                Some(OpCode::GtF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a > b)?; }
                Some(OpCode::LeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a <= b)?; }
                Some(OpCode::GeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a >= b)?; }

                // Int comparison
                Some(OpCode::EqI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a == b)?; }
                Some(OpCode::NeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a != b)?; }
                Some(OpCode::LtI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a < b)?; }
                Some(OpCode::GtI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a > b)?; }
                Some(OpCode::LeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a <= b)?; }
                Some(OpCode::GeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a >= b)?; }

                // Logical
                Some(OpCode::And) => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a && b)?; }
                Some(OpCode::Or) => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a || b)?; }
                Some(OpCode::Not) => { let v = self.pop_b()?; self.push_b(!v)?; }

                // Casts
                Some(OpCode::F32ToI32) => { let v = self.pop_f()?; self.push_i(v as i32)?; }
                Some(OpCode::I32ToF32) => { let v = self.pop_i()?; self.push_f(v as f32)?; }

                // Control flow
                Some(OpCode::Jump) => {
                    pc = self.read_u32(&mut pc) as usize;
                }
                Some(OpCode::JumpIfFalse) => {
                    let target = self.read_u32(&mut pc) as usize;
                    let cond = self.pop_b()?;
                    if !cond {
                        pc = target;
                    }
                }

                // Math builtins
                Some(OpCode::Sin) => { let v = self.pop_f()?; self.push_f(v.sin())?; }
                Some(OpCode::Cos) => { let v = self.pop_f()?; self.push_f(v.cos())?; }
                Some(OpCode::Tan) => { let v = self.pop_f()?; self.push_f(v.tan())?; }
                Some(OpCode::Asin) => { let v = self.pop_f()?; self.push_f(v.asin())?; }
                Some(OpCode::Acos) => { let v = self.pop_f()?; self.push_f(v.acos())?; }
                Some(OpCode::Atan) => { let v = self.pop_f()?; self.push_f(v.atan())?; }
                Some(OpCode::Atan2) => { let x = self.pop_f()?; let y = self.pop_f()?; self.push_f(y.atan2(x))?; }
                Some(OpCode::Exp) => { let v = self.pop_f()?; self.push_f(v.exp())?; }
                Some(OpCode::Log) => { let v = self.pop_f()?; self.push_f(v.ln())?; }
                Some(OpCode::Log2) => { let v = self.pop_f()?; self.push_f(v.log2())?; }
                Some(OpCode::Pow) => { let e = self.pop_f()?; let b = self.pop_f()?; self.push_f(b.powf(e))?; }
                Some(OpCode::Sqrt) => { let v = self.pop_f()?; self.push_f(v.sqrt())?; }
                Some(OpCode::Floor) => { let v = self.pop_f()?; self.push_f(v.floor())?; }
                Some(OpCode::Ceil) => { let v = self.pop_f()?; self.push_f(v.ceil())?; }
                Some(OpCode::Round) => { let v = self.pop_f()?; self.push_f(v.round())?; }
                Some(OpCode::Trunc) => { let v = self.pop_f()?; self.push_f(v.trunc())?; }
                Some(OpCode::Fract) => { let v = self.pop_f()?; self.push_f(v.fract())?; }
                Some(OpCode::Abs) => { let v = self.pop_f()?; self.push_f(v.abs())?; }
                Some(OpCode::Sign) => { let v = self.pop_f()?; self.push_f(v.signum())?; }
                Some(OpCode::Clamp) => {
                    let hi = self.pop_f()?;
                    let lo = self.pop_f()?;
                    let v = self.pop_f()?;
                    self.push_f(v.clamp(lo, hi))?;
                }
                Some(OpCode::Min) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.min(b))?; }
                Some(OpCode::Max) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.max(b))?; }
                Some(OpCode::Mix) => {
                    let t = self.pop_f()?;
                    let b = self.pop_f()?;
                    let a = self.pop_f()?;
                    self.push_f(a + (b - a) * t)?;
                }
                Some(OpCode::Smoothstep) => {
                    let x = self.pop_f()?;
                    let e1 = self.pop_f()?;
                    let e0 = self.pop_f()?;
                    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    self.push_f(t * t * (3.0 - 2.0 * t))?;
                }
                Some(OpCode::IsNan) => {
                    let v = self.pop_f()?;
                    self.push_b(v.is_nan())?;
                }

                // Array length
                Some(OpCode::ArrayLen) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
                    let len = if arr_id < self.state_arrays.len() {
                        self.state_arrays[arr_id].len() as i32
                    } else {
                        0
                    };
                    self.push_i(len)?;
                }

                // Built-in constants
                Some(OpCode::LoadSampleRate) => {
                    self.push_i(sample_rate as i32)?;
                }
                Some(OpCode::LoadBufferSize) => {
                    self.push_i(buffer_size as i32)?;
                }

                // Draw/mouse opcodes are not valid in the audio ScriptVM
                Some(OpCode::DrawFillCircle) | Some(OpCode::DrawStrokeCircle) |
                Some(OpCode::DrawStrokeArc) | Some(OpCode::DrawLine) |
                Some(OpCode::DrawFillRect) | Some(OpCode::DrawStrokeRect) |
                Some(OpCode::MouseX) | Some(OpCode::MouseY) | Some(OpCode::MouseDown) |
                Some(OpCode::StoreParam) => {
                    return Err(ScriptError::InvalidOpcode(op));
                }

                None => return Err(ScriptError::InvalidOpcode(op)),
            }
        }

        Ok(())
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
    fn push_f(&mut self, v: f32) -> Result<(), ScriptError> {
        self.push(Value { f: v })
    }

    #[inline]
    fn push_i(&mut self, v: i32) -> Result<(), ScriptError> {
        self.push(Value { i: v })
    }

    #[inline]
    fn push_b(&mut self, v: bool) -> Result<(), ScriptError> {
        self.push(Value { b: v })
    }

    #[inline]
    fn pop(&mut self) -> Result<Value, ScriptError> {
        if self.sp == 0 {
            return Err(ScriptError::StackUnderflow);
        }
        self.sp -= 1;
        Ok(self.stack[self.sp])
    }

    #[inline]
    fn pop_f(&mut self) -> Result<f32, ScriptError> {
        Ok(unsafe { self.pop()?.f })
    }

    #[inline]
    fn pop_i(&mut self) -> Result<i32, ScriptError> {
        Ok(unsafe { self.pop()?.i })
    }

    #[inline]
    fn pop_b(&mut self) -> Result<bool, ScriptError> {
        Ok(unsafe { self.pop()?.b })
    }

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
    pub bytecode: Vec<u8>,
    pub constants_f32: Vec<f32>,
    pub constants_i32: Vec<i32>,
    stack: Vec<Value>,
    sp: usize,
    locals: Vec<Value>,
    pub params: Vec<f32>,
    pub state_scalars: Vec<Value>,
    pub state_arrays: Vec<Vec<f32>>,
    pub draw_commands: Vec<DrawCommand>,
    pub mouse: MouseState,
    instruction_limit: u64,
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
            draw_commands: Vec::new(),
            mouse: MouseState::default(),
            instruction_limit: 1_000_000, // lower limit for draw (runs per frame)
        }
    }

    /// Execute the draw bytecode. Call once per frame.
    /// Draw commands accumulate in `self.draw_commands` (cleared at start).
    pub fn execute(&mut self) -> Result<(), ScriptError> {
        self.sp = 0;
        self.draw_commands.clear();
        for l in &mut self.locals {
            *l = Value::default();
        }

        let mut pc: usize = 0;
        let mut ic: u64 = 0;
        let limit = self.instruction_limit;

        while pc < self.bytecode.len() {
            ic += 1;
            if ic > limit {
                return Err(ScriptError::ExecutionLimitExceeded);
            }

            let op = self.bytecode[pc];
            pc += 1;

            match OpCode::from_u8(op) {
                Some(OpCode::Halt) => return Ok(()),

                Some(OpCode::PushF32) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.push_f(self.constants_f32[idx])?;
                }
                Some(OpCode::PushI32) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.push_i(self.constants_i32[idx])?;
                }
                Some(OpCode::PushBool) => {
                    let v = self.bytecode[pc];
                    pc += 1;
                    self.push_b(v != 0)?;
                }
                Some(OpCode::Pop) => { self.pop()?; }

                Some(OpCode::LoadLocal) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.push(self.locals[idx])?;
                }
                Some(OpCode::StoreLocal) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.locals[idx] = self.pop()?;
                }
                Some(OpCode::LoadParam) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.push_f(self.params[idx])?;
                }
                Some(OpCode::LoadState) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.push(self.state_scalars[idx])?;
                }
                Some(OpCode::StoreState) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    self.state_scalars[idx] = self.pop()?;
                }
                Some(OpCode::LoadStateArray) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
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
                Some(OpCode::StoreStateArray) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
                    let val = unsafe { self.pop()?.f };
                    let idx = unsafe { self.pop()?.i };
                    if arr_id < self.state_arrays.len() {
                        let arr_len = self.state_arrays[arr_id].len();
                        let idx = ((idx % arr_len as i32) + arr_len as i32) as usize % arr_len;
                        self.state_arrays[arr_id][idx] = val;
                    }
                }
                Some(OpCode::ArrayLen) => {
                    let arr_id = self.read_u16(&mut pc) as usize;
                    let len = if arr_id < self.state_arrays.len() {
                        self.state_arrays[arr_id].len() as i32
                    } else {
                        0
                    };
                    self.push_i(len)?;
                }

                // Audio I/O not available in draw context
                Some(OpCode::LoadInput) | Some(OpCode::StoreOutput) => {
                    return Err(ScriptError::InvalidOpcode(op));
                }
                Some(OpCode::LoadSampleRate) | Some(OpCode::LoadBufferSize) => {
                    return Err(ScriptError::InvalidOpcode(op));
                }
                // Sample access not available in draw context
                Some(OpCode::SampleLen) | Some(OpCode::SampleRead) | Some(OpCode::SampleRateOf) => {
                    pc += 1; // skip slot byte
                    self.push_i(0)?;
                }

                // Float arithmetic
                Some(OpCode::AddF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a + b)?; }
                Some(OpCode::SubF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a - b)?; }
                Some(OpCode::MulF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a * b)?; }
                Some(OpCode::DivF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a / b } else { 0.0 })?; }
                Some(OpCode::ModF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(if b.abs() > 1e-30 { a % b } else { 0.0 })?; }
                Some(OpCode::NegF) => { let v = self.pop_f()?; self.push_f(-v)?; }

                // Int arithmetic
                Some(OpCode::AddI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_add(b))?; }
                Some(OpCode::SubI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_sub(b))?; }
                Some(OpCode::MulI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(a.wrapping_mul(b))?; }
                Some(OpCode::DivI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a / b } else { 0 })?; }
                Some(OpCode::ModI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_i(if b != 0 { a % b } else { 0 })?; }
                Some(OpCode::NegI) => { let v = self.pop_i()?; self.push_i(-v)?; }

                // Float comparison
                Some(OpCode::EqF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a == b)?; }
                Some(OpCode::NeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a != b)?; }
                Some(OpCode::LtF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a < b)?; }
                Some(OpCode::GtF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a > b)?; }
                Some(OpCode::LeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a <= b)?; }
                Some(OpCode::GeF) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_b(a >= b)?; }

                // Int comparison
                Some(OpCode::EqI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a == b)?; }
                Some(OpCode::NeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a != b)?; }
                Some(OpCode::LtI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a < b)?; }
                Some(OpCode::GtI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a > b)?; }
                Some(OpCode::LeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a <= b)?; }
                Some(OpCode::GeI) => { let b = self.pop_i()?; let a = self.pop_i()?; self.push_b(a >= b)?; }

                // Logical
                Some(OpCode::And) => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a && b)?; }
                Some(OpCode::Or) => { let b = self.pop_b()?; let a = self.pop_b()?; self.push_b(a || b)?; }
                Some(OpCode::Not) => { let v = self.pop_b()?; self.push_b(!v)?; }

                // Casts
                Some(OpCode::F32ToI32) => { let v = self.pop_f()?; self.push_i(v as i32)?; }
                Some(OpCode::I32ToF32) => { let v = self.pop_i()?; self.push_f(v as f32)?; }

                // Control flow
                Some(OpCode::Jump) => {
                    pc = self.read_u32(&mut pc) as usize;
                }
                Some(OpCode::JumpIfFalse) => {
                    let target = self.read_u32(&mut pc) as usize;
                    let cond = self.pop_b()?;
                    if !cond {
                        pc = target;
                    }
                }

                // Math builtins
                Some(OpCode::Sin) => { let v = self.pop_f()?; self.push_f(v.sin())?; }
                Some(OpCode::Cos) => { let v = self.pop_f()?; self.push_f(v.cos())?; }
                Some(OpCode::Tan) => { let v = self.pop_f()?; self.push_f(v.tan())?; }
                Some(OpCode::Asin) => { let v = self.pop_f()?; self.push_f(v.asin())?; }
                Some(OpCode::Acos) => { let v = self.pop_f()?; self.push_f(v.acos())?; }
                Some(OpCode::Atan) => { let v = self.pop_f()?; self.push_f(v.atan())?; }
                Some(OpCode::Atan2) => { let x = self.pop_f()?; let y = self.pop_f()?; self.push_f(y.atan2(x))?; }
                Some(OpCode::Exp) => { let v = self.pop_f()?; self.push_f(v.exp())?; }
                Some(OpCode::Log) => { let v = self.pop_f()?; self.push_f(v.ln())?; }
                Some(OpCode::Log2) => { let v = self.pop_f()?; self.push_f(v.log2())?; }
                Some(OpCode::Pow) => { let e = self.pop_f()?; let b = self.pop_f()?; self.push_f(b.powf(e))?; }
                Some(OpCode::Sqrt) => { let v = self.pop_f()?; self.push_f(v.sqrt())?; }
                Some(OpCode::Floor) => { let v = self.pop_f()?; self.push_f(v.floor())?; }
                Some(OpCode::Ceil) => { let v = self.pop_f()?; self.push_f(v.ceil())?; }
                Some(OpCode::Round) => { let v = self.pop_f()?; self.push_f(v.round())?; }
                Some(OpCode::Trunc) => { let v = self.pop_f()?; self.push_f(v.trunc())?; }
                Some(OpCode::Fract) => { let v = self.pop_f()?; self.push_f(v.fract())?; }
                Some(OpCode::Abs) => { let v = self.pop_f()?; self.push_f(v.abs())?; }
                Some(OpCode::Sign) => { let v = self.pop_f()?; self.push_f(v.signum())?; }
                Some(OpCode::Clamp) => {
                    let hi = self.pop_f()?;
                    let lo = self.pop_f()?;
                    let v = self.pop_f()?;
                    self.push_f(v.clamp(lo, hi))?;
                }
                Some(OpCode::Min) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.min(b))?; }
                Some(OpCode::Max) => { let b = self.pop_f()?; let a = self.pop_f()?; self.push_f(a.max(b))?; }
                Some(OpCode::Mix) => {
                    let t = self.pop_f()?;
                    let b = self.pop_f()?;
                    let a = self.pop_f()?;
                    self.push_f(a + (b - a) * t)?;
                }
                Some(OpCode::Smoothstep) => {
                    let x = self.pop_f()?;
                    let e1 = self.pop_f()?;
                    let e0 = self.pop_f()?;
                    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    self.push_f(t * t * (3.0 - 2.0 * t))?;
                }
                Some(OpCode::IsNan) => { let v = self.pop_f()?; self.push_b(v.is_nan())?; }

                // Draw commands
                Some(OpCode::DrawFillCircle) => {
                    let color = self.pop_i()? as u32;
                    let r = self.pop_f()?;
                    let cy = self.pop_f()?;
                    let cx = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::FillCircle { cx, cy, r, color });
                }
                Some(OpCode::DrawStrokeCircle) => {
                    let width = self.pop_f()?;
                    let color = self.pop_i()? as u32;
                    let r = self.pop_f()?;
                    let cy = self.pop_f()?;
                    let cx = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::StrokeCircle { cx, cy, r, color, width });
                }
                Some(OpCode::DrawStrokeArc) => {
                    let width = self.pop_f()?;
                    let color = self.pop_i()? as u32;
                    let end_deg = self.pop_f()?;
                    let start_deg = self.pop_f()?;
                    let r = self.pop_f()?;
                    let cy = self.pop_f()?;
                    let cx = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::StrokeArc { cx, cy, r, start_deg, end_deg, color, width });
                }
                Some(OpCode::DrawLine) => {
                    let width = self.pop_f()?;
                    let color = self.pop_i()? as u32;
                    let y2 = self.pop_f()?;
                    let x2 = self.pop_f()?;
                    let y1 = self.pop_f()?;
                    let x1 = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::Line { x1, y1, x2, y2, color, width });
                }
                Some(OpCode::DrawFillRect) => {
                    let color = self.pop_i()? as u32;
                    let h = self.pop_f()?;
                    let w = self.pop_f()?;
                    let y = self.pop_f()?;
                    let x = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::FillRect { x, y, w, h, color });
                }
                Some(OpCode::DrawStrokeRect) => {
                    let width = self.pop_f()?;
                    let color = self.pop_i()? as u32;
                    let h = self.pop_f()?;
                    let w = self.pop_f()?;
                    let y = self.pop_f()?;
                    let x = self.pop_f()?;
                    self.draw_commands.push(DrawCommand::StrokeRect { x, y, w, h, color, width });
                }

                // Mouse input
                Some(OpCode::MouseX) => { self.push_f(self.mouse.x)?; }
                Some(OpCode::MouseY) => { self.push_f(self.mouse.y)?; }
                Some(OpCode::MouseDown) => { self.push_f(if self.mouse.down { 1.0 } else { 0.0 })?; }

                // Param write
                Some(OpCode::StoreParam) => {
                    let idx = self.read_u16(&mut pc) as usize;
                    let val = self.pop_f()?;
                    if idx < self.params.len() {
                        self.params[idx] = val;
                    }
                }

                None => return Err(ScriptError::InvalidOpcode(op)),
            }
        }

        Ok(())
    }

    // Stack helpers (identical to ScriptVM)
    #[inline]
    fn push(&mut self, v: Value) -> Result<(), ScriptError> {
        if self.sp >= STACK_SIZE { return Err(ScriptError::StackOverflow); }
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
        if self.sp == 0 { return Err(ScriptError::StackUnderflow); }
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
