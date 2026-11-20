//! Spec-compliant pure-Rust BLAKE3 (256-bit) implementation for the T1 spike.
//!
//! Avoids adding an external `blake3` crate to the workspace `Cargo.lock`
//! during the throwaway spike while matching the exact BLAKE3 specification
//! (single-chunk and multi-chunk Merkle tree).

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;

const IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

#[inline(always)]
fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

#[inline(always)]
fn round(state: &mut [u32; 16], m: &[u32; 16]) {
    g(state, 0, 4, 8, 12, m[0], m[1]);
    g(state, 1, 5, 9, 13, m[2], m[3]);
    g(state, 2, 6, 10, 14, m[4], m[5]);
    g(state, 3, 7, 11, 15, m[6], m[7]);
    g(state, 0, 5, 10, 15, m[8], m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8, 13, m[12], m[13]);
    g(state, 3, 4, 9, 14, m[14], m[15]);
}

#[inline(always)]
fn permute(m: &mut [u32; 16]) {
    let mut permuted = [0u32; 16];
    for i in 0..16 {
        permuted[i] = m[MSG_PERMUTATION[i]];
    }
    *m = permuted;
}

fn compress(
    chaining_value: &[u32; 8],
    block_words: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    let mut state = [
        chaining_value[0],
        chaining_value[1],
        chaining_value[2],
        chaining_value[3],
        chaining_value[4],
        chaining_value[5],
        chaining_value[6],
        chaining_value[7],
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];
    let mut block = *block_words;
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);
    permute(&mut block);
    round(&mut state, &block);

    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }
    state
}

fn first_8_words(out: [u32; 16]) -> [u32; 8] {
    out[0..8].try_into().expect("8 words")
}

fn words_from_le_bytes(bytes: &[u8]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(4)) {
        *word = u32::from_le_bytes(chunk.try_into().expect("4 bytes"));
    }
    words
}

struct Output {
    input_cv: [u32; 8],
    block_words: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        first_8_words(compress(
            &self.input_cv,
            &self.block_words,
            self.counter,
            self.block_len,
            self.flags,
        ))
    }

    fn root_hash(&self) -> [u8; OUT_LEN] {
        let words = compress(
            &self.input_cv,
            &self.block_words,
            0,
            self.block_len,
            self.flags | ROOT,
        );
        let mut out = [0u8; OUT_LEN];
        for (i, word) in words[..8].iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

struct ChunkState {
    cv: [u32; 8],
    chunk_counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: u8,
    blocks_compressed: u8,
}

impl ChunkState {
    fn new(chunk_counter: u64) -> Self {
        Self {
            cv: IV,
            chunk_counter,
            block: [0u8; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.blocks_compressed as usize + self.block_len as usize
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 {
            CHUNK_START
        } else {
            0
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len as usize == BLOCK_LEN {
                let words = words_from_le_bytes(&self.block);
                self.cv = first_8_words(compress(
                    &self.cv,
                    &words,
                    self.chunk_counter,
                    BLOCK_LEN as u32,
                    self.start_flag(),
                ));
                self.blocks_compressed += 1;
                self.block = [0u8; BLOCK_LEN];
                self.block_len = 0;
            }
            let want = BLOCK_LEN - self.block_len as usize;
            let take = want.min(input.len());
            self.block[self.block_len as usize..self.block_len as usize + take]
                .copy_from_slice(&input[..take]);
            self.block_len += take as u8;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        Output {
            input_cv: self.cv,
            block_words: words_from_le_bytes(&self.block),
            counter: self.chunk_counter,
            block_len: self.block_len as u32,
            flags: self.start_flag() | CHUNK_END,
        }
    }
}

fn parent_output(left_cv: [u32; 8], right_cv: [u32; 8]) -> Output {
    let mut block_words = [0u32; 16];
    block_words[..8].copy_from_slice(&left_cv);
    block_words[8..].copy_from_slice(&right_cv);
    Output {
        input_cv: IV,
        block_words,
        counter: 0,
        block_len: BLOCK_LEN as u32,
        flags: PARENT,
    }
}

/// Incremental BLAKE3 hasher.
pub struct Hasher {
    chunk_state: ChunkState,
    cv_stack: [[u32; 8]; 54],
    cv_stack_len: u8,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    pub fn new() -> Self {
        Self {
            chunk_state: ChunkState::new(0),
            cv_stack: [[0u32; 8]; 54],
            cv_stack_len: 0,
        }
    }

    fn push_stack(&mut self, cv: [u32; 8]) {
        self.cv_stack[self.cv_stack_len as usize] = cv;
        self.cv_stack_len += 1;
    }

    fn pop_stack(&mut self) -> [u32; 8] {
        self.cv_stack_len -= 1;
        self.cv_stack[self.cv_stack_len as usize]
    }

    fn add_chunk_cv(&mut self, mut new_cv: [u32; 8], mut total_chunks: u64) {
        while total_chunks & 1 == 0 {
            let left = self.pop_stack();
            new_cv = parent_output(left, new_cv).chaining_value();
            total_chunks >>= 1;
        }
        self.push_stack(new_cv);
    }

    pub fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.chunk_state.len() == CHUNK_LEN {
                let chunk_cv = self.chunk_state.output().chaining_value();
                let total_chunks = self.chunk_state.chunk_counter + 1;
                self.add_chunk_cv(chunk_cv, total_chunks);
                self.chunk_state = ChunkState::new(total_chunks);
            }
            let want = CHUNK_LEN - self.chunk_state.len();
            let take = want.min(input.len());
            self.chunk_state.update(&input[..take]);
            input = &input[take..];
        }
    }

    pub fn finalize(&self) -> [u8; OUT_LEN] {
        let mut output = self.chunk_state.output();
        let mut remaining = self.cv_stack_len as usize;
        while remaining > 0 {
            remaining -= 1;
            output = parent_output(self.cv_stack[remaining], output.chaining_value());
        }
        output.root_hash()
    }
}

/// Computes the 256-bit BLAKE3 digest of `data`.
pub fn hash(data: &[u8]) -> [u8; OUT_LEN] {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Formats a 32-byte digest as lowercase hex.
pub fn to_hex(digest: &[u8; OUT_LEN]) -> String {
    let mut out = String::with_capacity(OUT_LEN * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_official_blake3_vectors() {
        // Official BLAKE3 test vectors (empty input and "abc").
        assert_eq!(
            to_hex(&hash(b"")),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            to_hex(&hash(b"abc")),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        // Official BLAKE3 test vector for input length 3: [0, 1, 2].
        assert_eq!(
            to_hex(&hash(&[0, 1, 2])),
            "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f"
        );
    }

    #[test]
    fn streaming_matches_single_shot_across_merkle_boundaries() {
        let payload: Vec<u8> = (0..10_000usize).map(|i| (i % 251) as u8).collect();
        let oneshot = hash(&payload);
        let mut streaming = Hasher::new();
        for piece in payload.chunks(137) {
            streaming.update(piece);
        }
        assert_eq!(oneshot, streaming.finalize());
    }
}
