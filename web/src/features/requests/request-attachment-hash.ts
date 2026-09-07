const INITIAL = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
])

const ROUND = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
])

export async function sha256Blob(blob: Blob, chunkBytes = 4 * 1024 * 1024) {
  const hash = new Sha256()
  for (let offset = 0; offset < blob.size; offset += chunkBytes) {
    hash.update(new Uint8Array(await blob.slice(offset, offset + chunkBytes).arrayBuffer()))
  }
  return hash.digestHex()
}

class Sha256 {
  private readonly state = new Uint32Array(INITIAL)
  private readonly buffer = new Uint8Array(64)
  private buffered = 0
  private bytes = 0

  update(bytes: Uint8Array) {
    this.bytes += bytes.length
    let offset = 0
    if (this.buffered > 0) {
      const copy = Math.min(64 - this.buffered, bytes.length)
      this.buffer.set(bytes.subarray(0, copy), this.buffered)
      this.buffered += copy
      offset += copy
      if (this.buffered === 64) {
        this.compress(this.buffer)
        this.buffered = 0
      }
    }
    while (offset + 64 <= bytes.length) {
      this.compress(bytes.subarray(offset, offset + 64))
      offset += 64
    }
    if (offset < bytes.length) {
      this.buffer.set(bytes.subarray(offset), 0)
      this.buffered = bytes.length - offset
    }
  }

  digestHex() {
    const tail = new Uint8Array(128)
    tail.set(this.buffer.subarray(0, this.buffered))
    tail[this.buffered] = 0x80
    const tailLength = this.buffered < 56 ? 64 : 128
    const bitLength = BigInt(this.bytes) * 8n
    const view = new DataView(tail.buffer)
    view.setUint32(tailLength - 8, Number(bitLength >> 32n), false)
    view.setUint32(tailLength - 4, Number(bitLength & 0xffffffffn), false)
    for (let offset = 0; offset < tailLength; offset += 64) {
      this.compress(tail.subarray(offset, offset + 64))
    }
    return [...this.state]
      .map((value) => value.toString(16).padStart(8, '0'))
      .join('')
  }

  private compress(block: Uint8Array) {
    const words = new Uint32Array(64)
    const view = new DataView(block.buffer, block.byteOffset, block.byteLength)
    for (let index = 0; index < 16; index += 1) {
      words[index] = view.getUint32(index * 4, false)
    }
    for (let index = 16; index < 64; index += 1) {
      const a = words[index - 15] ?? 0
      const b = words[index - 2] ?? 0
      const s0 = rotate(a, 7) ^ rotate(a, 18) ^ (a >>> 3)
      const s1 = rotate(b, 17) ^ rotate(b, 19) ^ (b >>> 10)
      words[index] = ((words[index - 16] ?? 0) + s0 + (words[index - 7] ?? 0) + s1) >>> 0
    }

    let [a, b, c, d, e, f, g, h] = this.state
    for (let index = 0; index < 64; index += 1) {
      const sum1 = rotate(e, 6) ^ rotate(e, 11) ^ rotate(e, 25)
      const choice = (e & f) ^ (~e & g)
      const t1 = (h + sum1 + choice + (ROUND[index] ?? 0) + (words[index] ?? 0)) >>> 0
      const sum0 = rotate(a, 2) ^ rotate(a, 13) ^ rotate(a, 22)
      const majority = (a & b) ^ (a & c) ^ (b & c)
      const t2 = (sum0 + majority) >>> 0
      h = g
      g = f
      f = e
      e = (d + t1) >>> 0
      d = c
      c = b
      b = a
      a = (t1 + t2) >>> 0
    }
    this.state[0] = ((this.state[0] ?? 0) + a) >>> 0
    this.state[1] = ((this.state[1] ?? 0) + b) >>> 0
    this.state[2] = ((this.state[2] ?? 0) + c) >>> 0
    this.state[3] = ((this.state[3] ?? 0) + d) >>> 0
    this.state[4] = ((this.state[4] ?? 0) + e) >>> 0
    this.state[5] = ((this.state[5] ?? 0) + f) >>> 0
    this.state[6] = ((this.state[6] ?? 0) + g) >>> 0
    this.state[7] = ((this.state[7] ?? 0) + h) >>> 0
  }
}

function rotate(value: number, bits: number) {
  return (value >>> bits) | (value << (32 - bits))
}
