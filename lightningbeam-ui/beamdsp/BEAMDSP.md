# BeamDSP Language Reference

BeamDSP is a domain-specific language for writing audio processing scripts in Lightningbeam. Scripts are compiled to bytecode and run on the real-time audio thread with guaranteed bounded execution time and constant memory usage.

## Quick Start

```
name "Simple Gain"
category effect

inputs {
    audio_in: audio
}

outputs {
    audio_out: audio
}

params {
    gain: 1.0 [0.0, 2.0] ""
}

process {
    for i in 0..buffer_size {
        audio_out[i * 2] = audio_in[i * 2] * gain;
        audio_out[i * 2 + 1] = audio_in[i * 2 + 1] * gain;
    }
}
```

Save this as a `.bdsp` file or create it directly in the Script Editor pane.

## Script Structure

A BeamDSP script is composed of **header blocks** followed by a **process block**. All blocks are optional except `name`, `category`, and `process`.

```
name "Display Name"
category effect|generator|utility

inputs { ... }
outputs { ... }
params { ... }
state { ... }
ui { ... }
process { ... }
```

### name

```
name "My Effect"
```

Sets the display name shown in the node graph.

### category

```
category effect
```

One of:
- **`effect`** — Processes audio (has inputs and outputs)
- **`generator`** — Produces audio or CV (outputs only, no audio inputs)
- **`utility`** — Signal routing, mixing, or other utility functions

### inputs

Declares input ports. Each input has a name and signal type.

```
inputs {
    audio_in: audio
    mod_signal: cv
}
```

Signal types:
- **`audio`** — Stereo interleaved audio (2 samples per frame: left, right)
- **`cv`** — Mono control voltage (1 sample per frame, NaN when unconnected)

### outputs

Declares output ports. Same syntax as inputs.

```
outputs {
    audio_out: audio
    env_out: cv
}
```

### params

Declares user-adjustable parameters. Each parameter has a default value, range, and unit string.

```
params {
    frequency: 440.0 [20.0, 20000.0] "Hz"
    gain:      1.0   [0.0, 2.0]      ""
    mix:       0.5   [0.0, 1.0]      ""
}
```

Format: `name: default [min, max] "unit"`

Parameters appear as sliders in the node's UI. They are read-only inside the `process` block.

### state

Declares persistent variables that survive across process calls. State is zero-initialized and can be reset.

```
state {
    phase: f32
    counter: int
    active: bool
    buffer: [44100]f32
    indices: [16]int
    clip: sample
}
```

Types:
| Type | Description |
|------|-------------|
| `f32` | 32-bit float |
| `int` | 32-bit signed integer |
| `bool` | Boolean |
| `[N]f32` | Fixed-size float array (N is a constant) |
| `[N]int` | Fixed-size integer array (N is a constant) |
| `sample` | Loadable audio sample (stereo interleaved, read-only in process) |

State arrays are allocated once at compile time and never resized. The `sample` type holds audio data loaded through the node's UI.

### ui

Declares the layout of controls rendered below the node in the graph editor. If omitted, a default UI is generated with sliders for all parameters and pickers for all samples.

```
ui {
    sample clip
    param frequency
    param gain
    group "Mix" {
        param mix
    }
}
```

Elements:
| Element | Description |
|---------|-------------|
| `param name` | Slider for the named parameter |
| `sample name` | Audio clip picker for the named sample state variable |
| `group "label" { ... }` | Collapsible section containing child elements |

### process

The process block runs once per audio callback, processing all frames in the current buffer.

```
process {
    for i in 0..buffer_size {
        audio_out[i * 2] = audio_in[i * 2];
        audio_out[i * 2 + 1] = audio_in[i * 2 + 1];
    }
}
```

## Types

BeamDSP has three scalar types:

| Type | Description | Literal examples |
|------|-------------|-----------------|
| `f32` | 32-bit float | `1.0`, `0.5`, `3.14` |
| `int` | 32-bit signed integer | `0`, `42`, `256` |
| `bool` | Boolean | `true`, `false` |

Type conversions use cast syntax:
- `int(expr)` — Convert float to integer (truncates toward zero)
- `float(expr)` — Convert integer to float

Arithmetic between `int` and `f32` promotes the result to `f32`.

## Variables

### Local variables

```
let x = 1.0;
let mut counter = 0;
```

Use `let` to declare a local variable. Add `mut` to allow reassignment. Local variables exist only within the current block scope.

### Built-in variables

| Variable | Type | Description |
|----------|------|-------------|
| `sample_rate` | `int` | Audio sample rate in Hz (e.g., 44100) |
| `buffer_size` | `int` | Number of frames in the current buffer |

### Inputs and outputs

Input and output ports are accessed as arrays:

```
// Audio is stereo interleaved: [L0, R0, L1, R1, ...]
let left  = audio_in[i * 2];
let right = audio_in[i * 2 + 1];
audio_out[i * 2]     = left;
audio_out[i * 2 + 1] = right;

// CV is mono: one sample per frame
let mod_value = mod_in[i];
cv_out[i] = mod_value;
```

Input arrays are read-only. Output arrays are write-only.

### Parameters

Parameters are available as read-only `f32` variables:

```
audio_out[i * 2] = audio_in[i * 2] * gain;
```

### State variables

State scalars and arrays are mutable and persist across calls:

```
state {
    phase: f32
    buffer: [1024]f32
}

process {
    phase = phase + 0.01;
    buffer[0] = phase;
}
```

## Control Flow

### if / else

```
if phase >= 1.0 {
    phase = phase - 1.0;
}

if value > threshold {
    audio_out[i * 2] = 1.0;
} else {
    audio_out[i * 2] = 0.0;
}
```

### for loops

For loops iterate from 0 to an upper bound (exclusive). The loop variable is an immutable `int`.

```
for i in 0..buffer_size {
    // i goes from 0 to buffer_size - 1
}

for j in 0..len(buffer) {
    buffer[j] = 0.0;
}
```

The upper bound must be an integer expression. Typically `buffer_size`, `len(array)`, or a constant.

There are no `while` loops, no recursion, and no user-defined functions. This is by design — it guarantees bounded execution time on the audio thread.

## Operators

### Arithmetic
| Operator | Description |
|----------|-------------|
| `+` | Addition |
| `-` | Subtraction (binary) or negation (unary) |
| `*` | Multiplication |
| `/` | Division |
| `%` | Modulo |

### Comparison
| Operator | Description |
|----------|-------------|
| `==` | Equal |
| `!=` | Not equal |
| `<` | Less than |
| `>` | Greater than |
| `<=` | Less than or equal |
| `>=` | Greater than or equal |

### Logical
| Operator | Description |
|----------|-------------|
| `&&` | Logical AND |
| `\|\|` | Logical OR |
| `!` | Logical NOT (unary) |

## Built-in Functions

### Trigonometric
| Function | Description |
|----------|-------------|
| `sin(x)` | Sine |
| `cos(x)` | Cosine |
| `tan(x)` | Tangent |
| `asin(x)` | Arc sine |
| `acos(x)` | Arc cosine |
| `atan(x)` | Arc tangent |
| `atan2(y, x)` | Two-argument arc tangent |

### Exponential
| Function | Description |
|----------|-------------|
| `exp(x)` | e^x |
| `log(x)` | Natural logarithm |
| `log2(x)` | Base-2 logarithm |
| `pow(x, y)` | x raised to power y |
| `sqrt(x)` | Square root |

### Rounding
| Function | Description |
|----------|-------------|
| `floor(x)` | Round toward negative infinity |
| `ceil(x)` | Round toward positive infinity |
| `round(x)` | Round to nearest integer |
| `trunc(x)` | Round toward zero |
| `fract(x)` | Fractional part (x - floor(x)) |

### Clamping and interpolation
| Function | Description |
|----------|-------------|
| `abs(x)` | Absolute value |
| `sign(x)` | Sign (-1.0, 0.0, or 1.0) |
| `min(x, y)` | Minimum of two values |
| `max(x, y)` | Maximum of two values |
| `clamp(x, lo, hi)` | Clamp x to [lo, hi] |
| `mix(a, b, t)` | Linear interpolation: a*(1-t) + b*t |
| `smoothstep(edge0, edge1, x)` | Hermite interpolation between 0 and 1 |

### Array
| Function | Description |
|----------|-------------|
| `len(array)` | Length of a state array (returns `int`) |

### CV
| Function | Description |
|----------|-------------|
| `cv_or(value, default)` | Returns `default` if `value` is NaN (unconnected CV), otherwise returns `value` |

### Sample
| Function | Description |
|----------|-------------|
| `sample_len(s)` | Number of frames in sample (0 if unloaded, returns `int`) |
| `sample_read(s, index)` | Read sample data at index (0.0 if out of bounds, returns `f32`) |
| `sample_rate_of(s)` | Original sample rate of the loaded audio (returns `int`) |

Sample data is stereo interleaved, so frame N has left at index `N*2` and right at `N*2+1`.

## Comments

```
// This is a line comment
let x = 1.0; // Inline comment
```

Line comments start with `//` and extend to the end of the line.

## Semicolons

Semicolons are **optional** statement terminators. You can use them or omit them.

```
let x = 1.0;    // with semicolons
let y = 2.0

audio_out[0] = x + y
```

## Examples

### Stereo Delay

```
name "Stereo Delay"
category effect

inputs {
    audio_in: audio
}

outputs {
    audio_out: audio
}

params {
    delay_time: 0.5 [0.01, 2.0] "s"
    feedback:   0.3 [0.0, 0.95] ""
    mix:        0.5 [0.0, 1.0]  ""
}

state {
    buffer: [88200]f32
    write_pos: int
}

ui {
    param delay_time
    param feedback
    param mix
}

process {
    let delay_samples = int(delay_time * float(sample_rate)) * 2;
    for i in 0..buffer_size {
        let l = audio_in[i * 2];
        let r = audio_in[i * 2 + 1];
        let read_pos = (write_pos - delay_samples + len(buffer)) % len(buffer);
        let dl = buffer[read_pos];
        let dr = buffer[read_pos + 1];
        buffer[write_pos] = l + dl * feedback;
        buffer[write_pos + 1] = r + dr * feedback;
        write_pos = (write_pos + 2) % len(buffer);
        audio_out[i * 2]     = l * (1.0 - mix) + dl * mix;
        audio_out[i * 2 + 1] = r * (1.0 - mix) + dr * mix;
    }
}
```

### Sine Oscillator

```
name "Sine Oscillator"
category generator

outputs {
    audio_out: audio
}

params {
    frequency: 440.0 [20.0, 20000.0] "Hz"
    amplitude: 0.5   [0.0, 1.0]      ""
}

state {
    phase: f32
}

ui {
    param frequency
    param amplitude
}

process {
    let inc = frequency / float(sample_rate);
    for i in 0..buffer_size {
        let sample = sin(phase * 6.2831853) * amplitude;
        audio_out[i * 2] = sample;
        audio_out[i * 2 + 1] = sample;
        phase = phase + inc;
        if phase >= 1.0 {
            phase = phase - 1.0;
        }
    }
}
```

### Sample Player

```
name "One-Shot Player"
category generator

outputs {
    audio_out: audio
}

params {
    speed: 1.0 [0.1, 4.0] ""
}

state {
    clip: sample
    phase: f32
}

ui {
    sample clip
    param speed
}

process {
    let frames = sample_len(clip);
    for i in 0..buffer_size {
        let idx = int(phase) * 2;
        audio_out[i * 2] = sample_read(clip, idx);
        audio_out[i * 2 + 1] = sample_read(clip, idx + 1);
        phase = phase + speed;
        if phase >= float(frames) {
            phase = 0.0;
        }
    }
}
```

### CV-Controlled Filter (Tone Control)

```
name "Tone Control"
category effect

inputs {
    audio_in: audio
    cutoff_cv: cv
}

outputs {
    audio_out: audio
}

params {
    cutoff: 1000.0 [20.0, 20000.0] "Hz"
    resonance: 0.5 [0.0, 1.0] ""
}

state {
    lp_l: f32
    lp_r: f32
}

ui {
    param cutoff
    param resonance
}

process {
    for i in 0..buffer_size {
        let cv_mod = cv_or(cutoff_cv[i], 0.0);
        let freq = clamp(cutoff + cv_mod * 5000.0, 20.0, 20000.0);
        let rc = 1.0 / (6.2831853 * freq);
        let dt = 1.0 / float(sample_rate);
        let alpha = dt / (rc + dt);

        let l = audio_in[i * 2];
        let r = audio_in[i * 2 + 1];

        lp_l = lp_l + alpha * (l - lp_l);
        lp_r = lp_r + alpha * (r - lp_r);

        audio_out[i * 2] = lp_l;
        audio_out[i * 2 + 1] = lp_r;
    }
}
```

### LFO

```
name "LFO"
category generator

outputs {
    cv_out: cv
}

params {
    rate:  1.0 [0.01, 20.0] "Hz"
    depth: 1.0 [0.0, 1.0]   ""
}

state {
    phase: f32
}

ui {
    param rate
    param depth
}

process {
    let inc = rate / float(sample_rate);
    for i in 0..buffer_size {
        cv_out[i] = sin(phase * 6.2831853) * depth;
        phase = phase + inc;
        if phase >= 1.0 {
            phase = phase - 1.0;
        }
    }
}
```

## Safety Model

BeamDSP scripts run on the real-time audio thread. The language enforces safety through compile-time restrictions:

- **Bounded time**: Only `for i in 0..N` loops with statically bounded N. No `while` loops, no recursion, no user-defined functions. An instruction counter limit (~10 million) acts as a safety net.
- **Constant memory**: All state arrays have compile-time sizes. The VM uses a fixed-size stack (256 slots) and fixed locals (64 slots). No heap allocation occurs during processing.
- **Fail-silent**: If the VM encounters a runtime error (stack overflow, instruction limit exceeded), all outputs are zeroed for that buffer. Audio does not glitch — it simply goes silent.

## File Format

BeamDSP scripts use the `.bdsp` file extension. Files are plain UTF-8 text. You can export and import `.bdsp` files through the Script Editor pane or the node graph's script picker dropdown.
