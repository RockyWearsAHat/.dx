/**
 * `dxz1` — the document platform's own byte compressor.
 *
 * A self-contained LZSS (Lempel–Ziv sliding-window) codec written in pure TypeScript
 * with zero dependencies and no `node:` imports, so bundles compress and decompress
 * identically on Node, Deno, Bun, browsers, and edge runtimes. Because the platform
 * owns both ends of the format, the codec is not required to be compatible with DEFLATE
 * or brotli — only to round-trip exactly: `decompress(compress(x))` deep-equals `x` for
 * every byte input.
 *
 * Frame layout:
 *   [0..3]  magic  'DXZ1'
 *   [4..7]  original byte length (uint32, big-endian)
 *   [8..]   token stream
 *
 * Token stream: a flag byte precedes each group of up to 8 tokens; bit (7 - i), MSB
 * first, marks token i as a back-reference (1) or a literal (0).
 *   literal        → 1 byte: the original byte.
 *   back-reference → 3 bytes: [length - MIN_MATCH][distance >> 8][distance & 0xff],
 *                    copying `length` bytes from `distance` positions earlier (overlap
 *                    permitted, which encodes runs).
 */

const MAGIC = 0x44585a31; // 'DXZ1'
const HEADER_LENGTH = 8;

const MIN_MATCH = 3;
const MAX_MATCH = 258; // length byte 0..255 maps to MIN_MATCH..258
const MAX_DISTANCE = 0xffff; // distance fits in two bytes
const HASH_SIZE = 1 << 15;
const HASH_MASK = HASH_SIZE - 1;
const MAX_CHAIN = 128; // bound match search so compression stays linear-ish

/** Hash the 3-byte sequence at `pos` into the chain-table index space. */
function hashAt(input: Uint8Array, pos: number): number {
  return (((input[pos] << 10) ^ (input[pos + 1] << 5) ^ input[pos + 2]) & HASH_MASK) >>> 0;
}

/**
 * Compress raw bytes into a `dxz1` frame. Always succeeds and always round-trips; worst
 * case (incompressible input) the frame is the original plus ~12.5% flag overhead and
 * the 8-byte header.
 */
export function compress(input: Uint8Array): Uint8Array {
  const n = input.length;
  const out: number[] = [
    (MAGIC >>> 24) & 0xff, (MAGIC >>> 16) & 0xff, (MAGIC >>> 8) & 0xff, MAGIC & 0xff,
    (n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff,
  ];

  const head = new Int32Array(HASH_SIZE).fill(-1);
  const prev = new Int32Array(n > 0 ? n : 1).fill(-1);

  let flagIndex = out.length;
  out.push(0);
  let flagBits = 0;

  /** Append one token, managing the rolling flag byte. */
  const emit = (isMatch: boolean, bytes: number[]): void => {
    if (flagBits === 8) {
      flagIndex = out.length;
      out.push(0);
      flagBits = 0;
    }
    if (isMatch) {
      out[flagIndex] |= 1 << (7 - flagBits);
    }
    flagBits += 1;
    for (let i = 0; i < bytes.length; i += 1) {
      out.push(bytes[i]);
    }
  };

  /** Record the hash-chain entry for `pos` (requires a full 3-byte window). */
  const insert = (pos: number): void => {
    if (pos + MIN_MATCH <= n) {
      const h = hashAt(input, pos);
      prev[pos] = head[h];
      head[h] = pos;
    }
  };

  let pos = 0;
  while (pos < n) {
    let bestLen = MIN_MATCH - 1;
    let bestDist = 0;

    if (pos + MIN_MATCH <= n) {
      const maxLen = Math.min(MAX_MATCH, n - pos);
      let candidate = head[hashAt(input, pos)];
      let chain = MAX_CHAIN;
      while (candidate >= 0 && chain > 0) {
        const distance = pos - candidate;
        if (distance > MAX_DISTANCE) {
          break;
        }
        let len = 0;
        while (len < maxLen && input[candidate + len] === input[pos + len]) {
          len += 1;
        }
        if (len > bestLen) {
          bestLen = len;
          bestDist = distance;
          if (len === maxLen) {
            break;
          }
        }
        candidate = prev[candidate];
        chain -= 1;
      }
    }

    if (bestLen >= MIN_MATCH) {
      emit(true, [bestLen - MIN_MATCH, (bestDist >> 8) & 0xff, bestDist & 0xff]);
      const end = pos + bestLen;
      while (pos < end) {
        insert(pos);
        pos += 1;
      }
    } else {
      emit(false, [input[pos]]);
      insert(pos);
      pos += 1;
    }
  }

  return Uint8Array.from(out);
}

/** Read a big-endian uint32 from `bytes` at `offset`. */
function readUint32(bytes: Uint8Array, offset: number): number {
  return ((bytes[offset] << 24) | (bytes[offset + 1] << 16) | (bytes[offset + 2] << 8) | bytes[offset + 3]) >>> 0;
}

/**
 * Decompress a `dxz1` frame produced by {@link compress}. Throws if the magic is wrong
 * or the stream is truncated, rather than returning corrupt output.
 */
export function decompress(frame: Uint8Array): Uint8Array {
  if (frame.length < HEADER_LENGTH || readUint32(frame, 0) !== MAGIC) {
    throw new Error('dxz1: invalid frame magic');
  }

  const originalLength = readUint32(frame, 4);
  const out = new Uint8Array(originalLength);
  let outPos = 0;
  let i = HEADER_LENGTH;

  while (outPos < originalLength) {
    if (i >= frame.length) {
      throw new Error('dxz1: truncated frame (missing flag byte)');
    }
    const flag = frame[i];
    i += 1;

    for (let bit = 0; bit < 8 && outPos < originalLength; bit += 1) {
      const isMatch = (flag >> (7 - bit)) & 1;
      if (isMatch) {
        if (i + 3 > frame.length) {
          throw new Error('dxz1: truncated frame (missing match token)');
        }
        const length = frame[i] + MIN_MATCH;
        const distance = (frame[i + 1] << 8) | frame[i + 2];
        i += 3;
        const from = outPos - distance;
        if (from < 0 || outPos + length > originalLength) {
          throw new Error('dxz1: corrupt match token');
        }
        for (let k = 0; k < length; k += 1) {
          out[outPos] = out[from + k];
          outPos += 1;
        }
      } else {
        if (i >= frame.length) {
          throw new Error('dxz1: truncated frame (missing literal)');
        }
        out[outPos] = frame[i];
        outPos += 1;
        i += 1;
      }
    }
  }

  return out;
}
