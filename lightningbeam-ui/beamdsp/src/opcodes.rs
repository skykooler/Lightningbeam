/// Bytecode opcodes for the BeamDSP VM
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    // Stack operations
    PushF32 = 0,       // next 4 bytes: f32 constant index (u16)
    PushI32 = 1,       // next 2 bytes: i32 constant index (u16)
    PushBool = 2,      // next 1 byte: 0 or 1
    Pop = 3,

    // Variable access (all use u16 index)
    LoadLocal = 10,
    StoreLocal = 11,
    LoadParam = 12,
    LoadState = 13,
    StoreState = 14,

    // Buffer access (u8 port index)
    // LoadInput: pops index from stack, pushes input[port][index]
    LoadInput = 20,
    // StoreOutput: pops value then index, stores output[port][index] = value
    StoreOutput = 21,
    // State arrays (u16 array id)
    LoadStateArray = 22,   // pops index, pushes state_array[id][index]
    StoreStateArray = 23,  // pops value then index, stores state_array[id][index]

    // Sample access (u8 slot index)
    SampleLen = 25,        // pushes frame count
    SampleRead = 26,       // pops index, pushes sample data
    SampleRateOf = 27,     // pushes sample rate

    // Float arithmetic
    AddF = 30,
    SubF = 31,
    MulF = 32,
    DivF = 33,
    ModF = 34,
    NegF = 35,

    // Int arithmetic
    AddI = 40,
    SubI = 41,
    MulI = 42,
    DivI = 43,
    ModI = 44,
    NegI = 45,

    // Float comparison (push bool)
    EqF = 50,
    NeF = 51,
    LtF = 52,
    GtF = 53,
    LeF = 54,
    GeF = 55,

    // Int comparison (push bool)
    EqI = 60,
    NeI = 61,
    LtI = 62,
    GtI = 63,
    LeI = 64,
    GeI = 65,

    // Logical
    And = 70,
    Or = 71,
    Not = 72,

    // Type conversion
    F32ToI32 = 80,
    I32ToF32 = 81,

    // Control flow (u32 offset)
    Jump = 90,
    JumpIfFalse = 91,

    // Built-in math functions (operate on stack)
    Sin = 100,
    Cos = 101,
    Tan = 102,
    Asin = 103,
    Acos = 104,
    Atan = 105,
    Atan2 = 106,
    Exp = 107,
    Log = 108,
    Log2 = 109,
    Pow = 110,
    Sqrt = 111,
    Floor = 112,
    Ceil = 113,
    Round = 114,
    Trunc = 115,
    Fract = 116,
    Abs = 117,
    Clamp = 118,
    Min = 119,
    Max = 120,
    Sign = 121,
    Mix = 122,
    Smoothstep = 123,
    IsNan = 124,

    // Array/buffer info
    ArrayLen = 130,       // u16 array_id, pushes length as int

    // Built-in constants
    LoadSampleRate = 140,
    LoadBufferSize = 141,

    // Draw commands (pop args from stack, push to draw command buffer)
    DrawFillCircle = 150,     // pops: color(i32), r, cy, cx
    DrawStrokeCircle = 151,   // pops: width, color(i32), r, cy, cx
    DrawStrokeArc = 152,      // pops: width, color(i32), end_deg, start_deg, r, cy, cx
    DrawLine = 153,           // pops: width, color(i32), y2, x2, y1, x1
    DrawFillRect = 154,       // pops: color(i32), h, w, y, x
    DrawStrokeRect = 155,     // pops: width, color(i32), h, w, y, x

    // Mouse input (push onto stack)
    MouseX = 160,             // pushes canvas-relative X as f32
    MouseY = 161,             // pushes canvas-relative Y as f32
    MouseDown = 162,          // pushes 1.0 if pressed, 0.0 if not

    // Param write (draw context only)
    StoreParam = 170,         // u16 param index, pops value from stack

    Halt = 255,
}

impl OpCode {
    pub fn from_u8(v: u8) -> Option<OpCode> {
        // Safety: we validate the opcode values
        match v {
            0 => Some(OpCode::PushF32),
            1 => Some(OpCode::PushI32),
            2 => Some(OpCode::PushBool),
            3 => Some(OpCode::Pop),
            10 => Some(OpCode::LoadLocal),
            11 => Some(OpCode::StoreLocal),
            12 => Some(OpCode::LoadParam),
            13 => Some(OpCode::LoadState),
            14 => Some(OpCode::StoreState),
            20 => Some(OpCode::LoadInput),
            21 => Some(OpCode::StoreOutput),
            22 => Some(OpCode::LoadStateArray),
            23 => Some(OpCode::StoreStateArray),
            25 => Some(OpCode::SampleLen),
            26 => Some(OpCode::SampleRead),
            27 => Some(OpCode::SampleRateOf),
            30 => Some(OpCode::AddF),
            31 => Some(OpCode::SubF),
            32 => Some(OpCode::MulF),
            33 => Some(OpCode::DivF),
            34 => Some(OpCode::ModF),
            35 => Some(OpCode::NegF),
            40 => Some(OpCode::AddI),
            41 => Some(OpCode::SubI),
            42 => Some(OpCode::MulI),
            43 => Some(OpCode::DivI),
            44 => Some(OpCode::ModI),
            45 => Some(OpCode::NegI),
            50 => Some(OpCode::EqF),
            51 => Some(OpCode::NeF),
            52 => Some(OpCode::LtF),
            53 => Some(OpCode::GtF),
            54 => Some(OpCode::LeF),
            55 => Some(OpCode::GeF),
            60 => Some(OpCode::EqI),
            61 => Some(OpCode::NeI),
            62 => Some(OpCode::LtI),
            63 => Some(OpCode::GtI),
            64 => Some(OpCode::LeI),
            65 => Some(OpCode::GeI),
            70 => Some(OpCode::And),
            71 => Some(OpCode::Or),
            72 => Some(OpCode::Not),
            80 => Some(OpCode::F32ToI32),
            81 => Some(OpCode::I32ToF32),
            90 => Some(OpCode::Jump),
            91 => Some(OpCode::JumpIfFalse),
            100 => Some(OpCode::Sin),
            101 => Some(OpCode::Cos),
            102 => Some(OpCode::Tan),
            103 => Some(OpCode::Asin),
            104 => Some(OpCode::Acos),
            105 => Some(OpCode::Atan),
            106 => Some(OpCode::Atan2),
            107 => Some(OpCode::Exp),
            108 => Some(OpCode::Log),
            109 => Some(OpCode::Log2),
            110 => Some(OpCode::Pow),
            111 => Some(OpCode::Sqrt),
            112 => Some(OpCode::Floor),
            113 => Some(OpCode::Ceil),
            114 => Some(OpCode::Round),
            115 => Some(OpCode::Trunc),
            116 => Some(OpCode::Fract),
            117 => Some(OpCode::Abs),
            118 => Some(OpCode::Clamp),
            119 => Some(OpCode::Min),
            120 => Some(OpCode::Max),
            121 => Some(OpCode::Sign),
            122 => Some(OpCode::Mix),
            123 => Some(OpCode::Smoothstep),
            124 => Some(OpCode::IsNan),
            130 => Some(OpCode::ArrayLen),
            140 => Some(OpCode::LoadSampleRate),
            141 => Some(OpCode::LoadBufferSize),
            150 => Some(OpCode::DrawFillCircle),
            151 => Some(OpCode::DrawStrokeCircle),
            152 => Some(OpCode::DrawStrokeArc),
            153 => Some(OpCode::DrawLine),
            154 => Some(OpCode::DrawFillRect),
            155 => Some(OpCode::DrawStrokeRect),
            160 => Some(OpCode::MouseX),
            161 => Some(OpCode::MouseY),
            162 => Some(OpCode::MouseDown),
            170 => Some(OpCode::StoreParam),
            255 => Some(OpCode::Halt),
            _ => None,
        }
    }
}
