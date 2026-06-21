//! Streaming min/max waveform LOD pyramid.
//!
//! A waveform pyramid is a tree of zoom levels. **Index = tree depth:**
//! `levels[0]` is the **root** (a single texel — the min/max envelope of the
//! whole file, lowest resolution); each deeper level is `BRANCH`× finer, and
//! `levels.last()` is the **floor** (one texel per `floor_samples_per_texel`
//! source frames — the finest *persisted* level). A node's children live at
//! `index + 1`, so the residency invariant ("a node is cleared only after its
//! children") reads straight off the index.
//!
//! Below the floor (finer than the floor bucket) is *not* stored; the caller
//! re-decodes the source window on demand for true per-sample detail.
//!
//! The builder is **streaming**: samples are pushed once, in order, and only the
//! finest level is accumulated (~`total_frames / floor` texels); the coarser
//! levels are derived by repeated `BRANCH:1` min/max reduction in [`finish`].
//! This yields the identical pyramid to an in-stream cascade (each parent = the
//! min/max of its children) without ever holding the full sample buffer.
//!
//! **Ragged edges are handled by reducing over available children:** a bucket
//! whose group is partial (1..BRANCH children, or `< floor` samples at the floor)
//! simply takes the min/max of what's there — no value padding. Padding to a
//! regular shape, if ever needed, is a GPU-texture/tile concern, not the data's.
//!
//! Each texel carries per-channel min/max for up to two channels
//! (`Lmin,Lmax,Rmin,Rmax`), matching the GPU waveform texture; mono mirrors the
//! left channel into the right.
//!
//! [`finish`]: WaveformPyramidBuilder::finish

/// Reduction factor between adjacent pyramid levels.
pub const BRANCH: u32 = 4;

/// Default finest-level resolution (source frames per floor texel). Trades
/// on-disk pyramid size against how soon zoom-in must re-decode the source.
pub const DEFAULT_FLOOR_SAMPLES_PER_TEXEL: u32 = 256;

/// One waveform texel: per-channel min/max (stereo; mono duplicates left→right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texel {
    pub l_min: f32,
    pub l_max: f32,
    pub r_min: f32,
    pub r_max: f32,
}

impl Texel {
    const EMPTY: Texel = Texel {
        l_min: f32::INFINITY,
        l_max: f32::NEG_INFINITY,
        r_min: f32::INFINITY,
        r_max: f32::NEG_INFINITY,
    };

    #[inline]
    fn include_sample(&mut self, l: f32, r: f32) {
        self.l_min = self.l_min.min(l);
        self.l_max = self.l_max.max(l);
        self.r_min = self.r_min.min(r);
        self.r_max = self.r_max.max(r);
    }

    #[inline]
    fn include_texel(&mut self, c: &Texel) {
        self.l_min = self.l_min.min(c.l_min);
        self.l_max = self.l_max.max(c.l_max);
        self.r_min = self.r_min.min(c.r_min);
        self.r_max = self.r_max.max(c.r_max);
    }
}

/// A built min/max LOD pyramid, **root-first**: `levels[0]` is the coarsest
/// (whole-file envelope), `levels.last()` is the finest persisted (floor).
#[derive(Clone, Debug)]
pub struct WaveformPyramid {
    pub floor_samples_per_texel: u32,
    pub branch: u32,
    pub channels: u32,
    pub total_frames: u64,
    pub levels: Vec<Vec<Texel>>,
}

impl WaveformPyramid {
    /// Coarsest level — a single texel (whole-file envelope), or empty if no
    /// samples were pushed.
    pub fn root(&self) -> &[Texel] {
        self.levels.first().map_or(&[][..], |v| v)
    }

    /// Finest persisted level (`floor_samples_per_texel` frames per texel).
    pub fn floor(&self) -> &[Texel] {
        self.levels.last().map_or(&[][..], |v| v)
    }

    /// Number of levels (tree depth + 1).
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// Serialize to a compact binary blob (for persisting in the `.beam`
    /// container). Header carries `B`/branch/channels/total_frames + per-level
    /// lengths, then root-first texel data (`f32` min/max).
    pub fn to_bytes(&self) -> Vec<u8> {
        let total_texels: usize = self.levels.iter().map(|l| l.len()).sum();
        let mut out = Vec::with_capacity(32 + self.levels.len() * 4 + total_texels * 16);
        out.extend_from_slice(b"LBWF");
        out.extend_from_slice(&1u32.to_le_bytes()); // format version
        out.extend_from_slice(&self.floor_samples_per_texel.to_le_bytes());
        out.extend_from_slice(&self.branch.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        out.extend_from_slice(&self.total_frames.to_le_bytes());
        out.extend_from_slice(&(self.levels.len() as u32).to_le_bytes());
        for level in &self.levels {
            out.extend_from_slice(&(level.len() as u32).to_le_bytes());
        }
        for level in &self.levels {
            for t in level {
                out.extend_from_slice(&t.l_min.to_le_bytes());
                out.extend_from_slice(&t.l_max.to_le_bytes());
                out.extend_from_slice(&t.r_min.to_le_bytes());
                out.extend_from_slice(&t.r_max.to_le_bytes());
            }
        }
        out
    }

    /// Reconstruct from [`WaveformPyramid::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<WaveformPyramid, String> {
        let mut r = ByteReader::new(data);
        if r.take(4)? != b"LBWF" {
            return Err("Not a waveform pyramid blob".to_string());
        }
        let version = r.u32()?;
        if version != 1 {
            return Err(format!("Unsupported waveform pyramid version {}", version));
        }
        let floor_samples_per_texel = r.u32()?;
        let branch = r.u32()?;
        let channels = r.u32()?;
        let total_frames = r.u64()?;
        let num_levels = r.u32()? as usize;
        let mut level_lens = Vec::with_capacity(num_levels);
        for _ in 0..num_levels {
            level_lens.push(r.u32()? as usize);
        }
        let mut levels = Vec::with_capacity(num_levels);
        for &len in &level_lens {
            let mut level = Vec::with_capacity(len);
            for _ in 0..len {
                level.push(Texel {
                    l_min: r.f32()?,
                    l_max: r.f32()?,
                    r_min: r.f32()?,
                    r_max: r.f32()?,
                });
            }
            levels.push(level);
        }
        Ok(WaveformPyramid {
            floor_samples_per_texel,
            branch,
            channels,
            total_frames,
            levels,
        })
    }
}

/// Minimal little-endian byte cursor for [`WaveformPyramid::from_bytes`].
struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("overflow")?;
        if end > self.data.len() {
            return Err("Truncated waveform pyramid blob".to_string());
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

/// Streaming builder for a [`WaveformPyramid`]. See the module docs.
pub struct WaveformPyramidBuilder {
    floor: u32,
    branch: u32,
    channels: u32,
    total_frames: u64,
    floor_level: Vec<Texel>,
    acc: Texel,
    acc_count: u32,
}

impl WaveformPyramidBuilder {
    pub fn new(channels: u32, floor_samples_per_texel: u32) -> Self {
        Self {
            floor: floor_samples_per_texel.max(1),
            branch: BRANCH,
            channels: channels.max(1),
            total_frames: 0,
            floor_level: Vec::new(),
            acc: Texel::EMPTY,
            acc_count: 0,
        }
    }

    /// Pre-reserve the floor `Vec` from an estimated total frame count (e.g. the
    /// probe's `total_frames`), to avoid reallocations during streaming. Purely a
    /// hint — the final size is set by the actual number of frames pushed.
    pub fn reserve_for_frames(&mut self, estimated_frames: u64) {
        let est_texels = (estimated_frames / self.floor as u64).saturating_add(1);
        self.floor_level.reserve(est_texels.min(usize::MAX as u64) as usize);
    }

    /// Push a block of interleaved samples (`channels` per frame). Partial
    /// trailing frames (fewer than `channels`) are ignored.
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        let ch = self.channels as usize;
        for frame in samples.chunks_exact(ch) {
            let l = frame[0];
            let r = if ch >= 2 { frame[1] } else { l };
            self.push_frame(l, r);
        }
    }

    #[inline]
    fn push_frame(&mut self, l: f32, r: f32) {
        self.total_frames += 1;
        self.acc.include_sample(l, r);
        self.acc_count += 1;
        if self.acc_count >= self.floor {
            self.floor_level.push(std::mem::replace(&mut self.acc, Texel::EMPTY));
            self.acc_count = 0;
        }
    }

    /// Flush the trailing partial bucket and reduce up to the root.
    pub fn finish(mut self) -> WaveformPyramid {
        if self.acc_count > 0 {
            self.floor_level.push(self.acc);
        }

        // Build finest-first by repeated BRANCH:1 reduction until one texel.
        // The shape is fully determined by the floor texel count; the last group
        // at each level may be ragged (1..BRANCH children) and reduces over what
        // it has.
        let mut levels = vec![std::mem::take(&mut self.floor_level)];
        let branch = self.branch as usize;
        while levels.last().map_or(0, |l| l.len()) > 1 {
            let prev = levels.last().unwrap();
            let mut next = Vec::with_capacity(prev.len().div_ceil(branch));
            for chunk in prev.chunks(branch) {
                let mut t = Texel::EMPTY;
                for c in chunk {
                    t.include_texel(c);
                }
                next.push(t);
            }
            levels.push(next);
        }
        // Output is root-first (convention B): levels[0] = root, last = floor.
        levels.reverse();

        WaveformPyramid {
            floor_samples_per_texel: self.floor,
            branch: self.branch,
            channels: self.channels,
            total_frames: self.total_frames,
            levels,
        }
    }
}

// Tests live in `daw-backend/tests/waveform_pyramid.rs` (integration tests) so
// they build the lib in normal mode, independent of the crate's pre-existing
// broken `#[cfg(test)]` unit tests (automation.rs).
