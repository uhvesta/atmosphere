import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
const [fixture, wasmPath, temporary] = process.argv.slice(2);
const checkpointPath = join(temporary, 'native-vt.checkpoint');
process.stdout.write(execFileSync(fixture, [checkpointPath]));
const checkpoint = readFileSync(checkpointPath);
assert.equal(checkpoint.subarray(0, 8).toString('hex'), '4756544300000001');
const body = checkpoint.subarray(20);
let instance;
({ instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {
  env: { log(ptr, len) { if (instance) process.stderr.write(new TextDecoder().decode(new Uint8Array(instance.exports.memory.buffer, ptr, len)) + '\n'); } },
}));
const e = instance.exports;
const view = () => new DataView(e.memory.buffer);
const memory = () => new Uint8Array(e.memory.buffer);
const check = (result) => assert.equal(result, 0, `Ghostty result ${result}`);
const allocation = (length) => { const ptr = e.ghostty_wasm_alloc(length); assert.notEqual(ptr, 0); return ptr; };
const opaque = e.ghostty_wasm_alloc_opaque();
assert.notEqual(opaque, 0);
const input = allocation(body.length);
memory().set(body, input);
const damaged = allocation(body.length);
memory().set(body, damaged);
memory()[damaged] ^= 1; // Invalid Ghostty envelope, with a live decoder to free.
const scalar = allocation(8);
const outPointer = allocation(4);
const outLength = allocation(4);
function readValue(terminal, field, size = 2) {
  check(e.ghostty_terminal_get(terminal, field, scalar));
  return size === 2 ? view().getUint16(scalar, true) : view().getUint32(scalar, true);
}
function feed(terminal, text) {
  const bytes = new TextEncoder().encode(text);
  const ptr = allocation(bytes.length);
  memory().set(bytes, ptr);
  e.ghostty_terminal_vt_write(terminal, ptr, bytes.length);
  e.ghostty_wasm_free(ptr, bytes.length);
}
function plain(terminal) {
  check(e.ghostty_formatter_terminal_new(0, opaque, terminal));
  const formatter = e.ghostty_wasm_take_opaque(opaque);
  try {
    check(e.ghostty_formatter_format_alloc(formatter, 0, outPointer, outLength));
    const ptr = view().getUint32(outPointer, true);
    const len = view().getUint32(outLength, true);
    const result = new TextDecoder().decode(new Uint8Array(e.memory.buffer, ptr, len));
    e.ghostty_free(0, ptr, len);
    return result;
  } finally { e.ghostty_formatter_free(formatter); }
}
const initialMemory = e.memory.buffer.byteLength;
const reversePath = join(temporary, 'wasm-vt.checkpoint');
let warmMemory;
const memorySamples = [];
for (let n = 0; n < 1000; n++) {
  check(e.ghostty_snapshot_decoder_new_buf(0, opaque, damaged, body.length));
  const failedDecoder = e.ghostty_wasm_take_opaque(opaque);
  try {
    assert.notEqual(e.ghostty_snapshot_decoder_decode(failedDecoder, opaque), 0);
    assert.equal(e.ghostty_wasm_take_opaque(opaque), 0);
  } finally { e.ghostty_snapshot_decoder_free(failedDecoder); }
  check(e.ghostty_snapshot_decoder_new_buf(0, opaque, input, body.length));
  const decoder = e.ghostty_wasm_take_opaque(opaque);
  let terminal;
  try {
    view().setUint32(scalar, 1024 * 1024, true);
    check(e.ghostty_snapshot_decoder_set(decoder, 1, scalar));
    memory()[scalar] = 1;
    check(e.ghostty_snapshot_decoder_set(decoder, 2, scalar));
    check(e.ghostty_snapshot_decoder_decode(decoder, opaque));
    terminal = e.ghostty_wasm_take_opaque(opaque);
  } finally { e.ghostty_snapshot_decoder_free(decoder); }
  try {
    assert.equal(readValue(terminal, 1), 40);
    assert.equal(readValue(terminal, 2), 6);
    assert.equal(readValue(terminal, 6, 4), 1, 'alternate screen must survive');
    // Finish the UTF-8 character split across the native checkpoint boundary.
    const tail = allocation(2);
    memory().set([0x91, 0xa9], tail);
    e.ghostty_terminal_vt_write(terminal, tail, 2);
    e.ghostty_wasm_free(tail, 2);
    assert.match(plain(terminal), /alternate link 👩/u);
    feed(terminal, '\x1b[?1049l');
    assert.equal(readValue(terminal, 6, 4), 0);
    assert.ok(readValue(terminal, 14, 4) >= 150, 'primary history must survive');
    const history = plain(terminal);
    assert.match(history, /history 0/);
    assert.match(history, /history 149/);
    if (n === 0) {
      feed(terminal, '\r\nwasm resumed');
      assert.equal(e.ghostty_snapshot_encode_buf(terminal, 0, 0, outLength), -3);
      const len = view().getUint32(outLength, true);
      const output = allocation(len);
      try {
        check(e.ghostty_snapshot_encode_buf(terminal, output, len, outLength));
        const envelope = Buffer.concat([checkpoint.subarray(0, 20), Buffer.from(memory().subarray(output, output + len))]);
        writeFileSync(reversePath, envelope);
      } finally { e.ghostty_wasm_free(output, len); }
    }
  } finally { e.ghostty_terminal_free(terminal); }
  if (n === 99) warmMemory = e.memory.buffer.byteLength;
  if ((n + 1) % 100 === 0) memorySamples.push(e.memory.buffer.byteLength);
}
const finalMemory = e.memory.buffer.byteLength;
assert.ok(finalMemory <= warmMemory + 65536, `Wasm heap failed to plateau after warmup: ${JSON.stringify(memorySamples)}`);
e.ghostty_wasm_free(input, body.length);
e.ghostty_wasm_free(damaged, body.length);
e.ghostty_wasm_free(scalar, 8);
e.ghostty_wasm_free(outPointer, 4);
e.ghostty_wasm_free(outLength, 4);
e.ghostty_wasm_free_opaque(opaque);
process.stdout.write(execFileSync(fixture, ['--verify', reversePath]));
console.log(JSON.stringify({ test: 'native64-to-wasm32-checkpoint', restores: 1000, malformedDecodes: 1000, initialMemory, warmMemory, finalMemory, memorySamples }));
