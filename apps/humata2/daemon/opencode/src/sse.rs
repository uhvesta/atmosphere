//! Splits OpenCode's server-sent event stream into the data of each frame.

/// Buffers the stream's bytes and returns the data of each frame they finish.
///
/// A frame ends at a blank line. Lines end in `\n` or `\r\n`, and a line that
/// arrives in pieces waits for the rest.
#[derive(Default)]
pub struct Frames {
    /// Bytes of the line still arriving.
    partial: Vec<u8>,
    /// The `data:` lines of the frame being read.
    lines: Vec<String>,
}

impl Frames {
    /// Adds `bytes` and returns the data of each frame they finish, in order.
    ///
    /// Comments, other fields and frames with no data are skipped. Invalid
    /// UTF-8 is replaced with the replacement character.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut buffer = std::mem::take(&mut self.partial);
        buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut start = 0;
        while let Some(offset) = buffer[start..].iter().position(|&byte| byte == b'\n') {
            let end = start + offset;
            self.line(&buffer[start..end], &mut frames);
            start = end + 1;
        }
        self.partial = buffer.split_off(start);
        frames
    }

    /// Reads one line without its `\n`, and finishes the frame on a blank line.
    fn line(&mut self, line: &[u8], frames: &mut Vec<String>) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            self.finish(frames);
        } else if let Some(value) = line.strip_prefix(b"data:") {
            let value = value.strip_prefix(b" ").unwrap_or(value);
            self.lines.push(String::from_utf8_lossy(value).into_owned());
        }
    }

    /// Ends the frame being read, keeping its data if it has any.
    fn finish(&mut self, frames: &mut Vec<String>) {
        let data = std::mem::take(&mut self.lines).join("\n");
        if !data.is_empty() {
            frames.push(data);
        }
    }
}
