/**
 * Runtime-agnostic byte helpers — the portable replacement for Node's `Buffer`.
 *
 * The document core builds and parses binary frames with these functions instead of
 * `Buffer`, so it runs unchanged on Node, Deno, Bun, browsers, and edge runtimes. All
 * multi-byte integers are little-endian unless a `BE` suffix says otherwise. Text is
 * always UTF-8.
 */

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

/** Encode a string as UTF-8 bytes. */
export function utf8Encode(text: string): Uint8Array {
  return textEncoder.encode(text);
}

/** Decode UTF-8 bytes (optionally a sub-range) to a string. */
export function utf8Decode(bytes: Uint8Array, start = 0, end = bytes.length): string {
  return textDecoder.decode(bytes.subarray(start, end));
}

/** Concatenate byte chunks into a single contiguous `Uint8Array`. */
export function concatBytes(chunks: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const chunk of chunks) {
    total += chunk.length;
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

/** Read a little-endian uint32, validating bounds. */
export function readUint32LE(bytes: Uint8Array, offset: number): number {
  if (offset + 4 > bytes.length) {
    throw new Error('readUint32LE: out of bounds');
  }
  return (
    (bytes[offset] | (bytes[offset + 1] << 8) | (bytes[offset + 2] << 16) | (bytes[offset + 3] << 24)) >>> 0
  );
}

/** Write a little-endian uint32 into `bytes` at `offset`. */
export function writeUint32LE(bytes: Uint8Array, offset: number, value: number): void {
  bytes[offset] = value & 0xff;
  bytes[offset + 1] = (value >>> 8) & 0xff;
  bytes[offset + 2] = (value >>> 16) & 0xff;
  bytes[offset + 3] = (value >>> 24) & 0xff;
}

/** Encode a uint32 as a fresh 4-byte little-endian array. */
export function uint32LE(value: number): Uint8Array {
  const out = new Uint8Array(4);
  writeUint32LE(out, 0, value);
  return out;
}

/** Render a sub-range of bytes as a lowercase hex string. */
export function toHex(bytes: Uint8Array, start = 0, end = bytes.length): string {
  let hex = '';
  for (let i = start; i < end; i += 1) {
    hex += bytes[i].toString(16).padStart(2, '0');
  }
  return hex;
}

/** True when two byte ranges are equal in length and content. */
export function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) {
    return false;
  }
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

/** True when `bytes` begins with `prefix`. */
export function startsWith(bytes: Uint8Array, prefix: Uint8Array): boolean {
  if (bytes.length < prefix.length) {
    return false;
  }
  for (let i = 0; i < prefix.length; i += 1) {
    if (bytes[i] !== prefix[i]) {
      return false;
    }
  }
  return true;
}
