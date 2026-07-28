/**
 * Runtime-agnostic cryptographic digests (SHA-256, SHA-1) over raw bytes or UTF-8 text.
 *
 * These are pure TypeScript implementations with zero dependencies and no `node:`
 * imports, so the document core hashes identically on Node, Deno, Bun, browsers, and
 * edge runtimes. Output is lowercase hex, byte-for-byte identical to Node's
 * `crypto.createHash('sha256'|'sha1').update(input).digest('hex')`.
 *
 * Strings are encoded as UTF-8 via the global `TextEncoder` (available in every modern
 * JavaScript runtime). `Uint8Array` / Node `Buffer` inputs are hashed as-is.
 */

const textEncoder = new TextEncoder();

/** Normalize a string-or-bytes input to a byte view (UTF-8 for strings). */
function toBytes(input: string | Uint8Array): Uint8Array {
  return typeof input === 'string' ? textEncoder.encode(input) : input;
}

/** Render bytes as a lowercase hex string. */
function toHex(bytes: Uint8Array): string {
  let hex = '';
  for (let i = 0; i < bytes.length; i += 1) {
    hex += bytes[i].toString(16).padStart(2, '0');
  }
  return hex;
}

/** Rotate a 32-bit word right by `n` bits. */
function rotr(value: number, bits: number): number {
  return ((value >>> bits) | (value << (32 - bits))) >>> 0;
}

/**
 * Pad a message to a 64-byte block boundary per the SHA-1/SHA-256 (Merkle–Damgård)
 * scheme: append 0x80, zero-fill, then the 64-bit big-endian bit length.
 */
function padMessage(message: Uint8Array): DataView {
  const length = message.length;
  const bitLength = length * 8;
  const withTerminator = length + 1;
  const totalLength = withTerminator + ((56 - (withTerminator % 64) + 64) % 64) + 8;

  const padded = new Uint8Array(totalLength);
  padded.set(message);
  padded[length] = 0x80;

  const view = new DataView(padded.buffer);
  view.setUint32(totalLength - 8, Math.floor(bitLength / 0x100000000));
  view.setUint32(totalLength - 4, bitLength >>> 0);
  return view;
}

const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

/** Compute the raw 32-byte SHA-256 digest of a byte message. */
function sha256Bytes(message: Uint8Array): Uint8Array {
  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);
  const view = padMessage(message);
  const totalLength = view.byteLength;
  const w = new Uint32Array(64);

  for (let offset = 0; offset < totalLength; offset += 64) {
    for (let i = 0; i < 16; i += 1) {
      w[i] = view.getUint32(offset + i * 4);
    }
    for (let i = 16; i < 64; i += 1) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
    }

    let a = h[0]; let b = h[1]; let c = h[2]; let d = h[3];
    let e = h[4]; let f = h[5]; let g = h[6]; let hh = h[7];

    for (let i = 0; i < 64; i += 1) {
      const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const t1 = (hh + s1 + ch + SHA256_K[i] + w[i]) >>> 0;
      const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (s0 + maj) >>> 0;
      hh = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }

    h[0] = (h[0] + a) >>> 0; h[1] = (h[1] + b) >>> 0; h[2] = (h[2] + c) >>> 0; h[3] = (h[3] + d) >>> 0;
    h[4] = (h[4] + e) >>> 0; h[5] = (h[5] + f) >>> 0; h[6] = (h[6] + g) >>> 0; h[7] = (h[7] + hh) >>> 0;
  }

  const out = new Uint8Array(32);
  const outView = new DataView(out.buffer);
  for (let i = 0; i < 8; i += 1) {
    outView.setUint32(i * 4, h[i]);
  }
  return out;
}

/** Compute the raw 20-byte SHA-1 digest of a byte message. */
function sha1Bytes(message: Uint8Array): Uint8Array {
  let h0 = 0x67452301; let h1 = 0xefcdab89; let h2 = 0x98badcfe; let h3 = 0x10325476; let h4 = 0xc3d2e1f0;
  const view = padMessage(message);
  const totalLength = view.byteLength;
  const w = new Uint32Array(80);

  for (let offset = 0; offset < totalLength; offset += 64) {
    for (let i = 0; i < 16; i += 1) {
      w[i] = view.getUint32(offset + i * 4);
    }
    for (let i = 16; i < 80; i += 1) {
      const v = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
      w[i] = ((v << 1) | (v >>> 31)) >>> 0;
    }

    let a = h0; let b = h1; let c = h2; let d = h3; let e = h4;
    for (let i = 0; i < 80; i += 1) {
      let f: number;
      let k: number;
      if (i < 20) { f = (b & c) | (~b & d); k = 0x5a827999; }
      else if (i < 40) { f = b ^ c ^ d; k = 0x6ed9eba1; }
      else if (i < 60) { f = (b & c) | (b & d) | (c & d); k = 0x8f1bbcdc; }
      else { f = b ^ c ^ d; k = 0xca62c1d6; }

      const temp = ((((a << 5) | (a >>> 27)) >>> 0) + f + e + k + w[i]) >>> 0;
      e = d; d = c; c = ((b << 30) | (b >>> 2)) >>> 0; b = a; a = temp;
    }

    h0 = (h0 + a) >>> 0; h1 = (h1 + b) >>> 0; h2 = (h2 + c) >>> 0; h3 = (h3 + d) >>> 0; h4 = (h4 + e) >>> 0;
  }

  const out = new Uint8Array(20);
  const outView = new DataView(out.buffer);
  outView.setUint32(0, h0); outView.setUint32(4, h1); outView.setUint32(8, h2);
  outView.setUint32(12, h3); outView.setUint32(16, h4);
  return out;
}

/** SHA-256 hex digest of UTF-8 text or raw bytes. */
export function sha256Hex(input: string | Uint8Array): string {
  return toHex(sha256Bytes(toBytes(input)));
}

/** SHA-1 hex digest of UTF-8 text or raw bytes. */
export function sha1Hex(input: string | Uint8Array): string {
  return toHex(sha1Bytes(toBytes(input)));
}
