// Internal bounded file encryption worker; all paths arrive from the owned backup wrapper.
import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { open, readFile, unlink } from 'node:fs/promises';
import { pipeline } from 'node:stream/promises';
import { Transform } from 'node:stream';

const aad = Buffer.from('lattice.customer-backup.aes-256-gcm.v1');
try {
  let input = '';
  for await (const chunk of process.stdin) {
    input += chunk;
    if (Buffer.byteLength(input) > 32 * 1024 * 1024) throw new Error('bounds');
  }
  const request = JSON.parse(input);
  if (!['encrypt', 'decrypt'].includes(request.action) || !Array.isArray(request.files) || request.files.length > 100000) throw new Error('request');
  const key = await readFile(request.key);
  if (key.length !== 32) throw new Error('key');
  for (const item of request.files) {
    let created = false;
    try {
      const output = await open(item.output, 'wx', 0o600);
      created = true;
      try {
        if (request.action === 'encrypt') {
          const nonce = randomBytes(12);
          const cipher = createCipheriv('aes-256-gcm', key, nonce, { authTagLength: 16 });
          cipher.setAAD(aad);
          await output.write(nonce);
          const digest = createHash('sha256'); let bytes = 0;
          const measured = new Transform({ transform(chunk, _, done) { digest.update(chunk); bytes += chunk.length; done(null, chunk); } });
          await pipeline(createReadStream(item.input), measured, cipher, createWriteStream(item.output, { fd: output.fd, autoClose: false, start: 12 }));
          if (digest.digest('hex') !== item.expected_sha256 || bytes !== item.expected_bytes) throw new Error('source changed');
          await output.write(cipher.getAuthTag(), 0, 16, (await output.stat()).size);
        } else {
          const source = await open(item.input, 'r');
          try {
            const size = (await source.stat()).size;
            if (size < 28) throw new Error('frame');
            const nonce = Buffer.alloc(12), tag = Buffer.alloc(16);
            await source.read(nonce, 0, 12, 0);
            await source.read(tag, 0, 16, size - 16);
            const cipher = createDecipheriv('aes-256-gcm', key, nonce, { authTagLength: 16 });
            cipher.setAAD(aad); cipher.setAuthTag(tag);
            if (size === 28) {
              await output.write(cipher.final());
            } else {
              await pipeline(createReadStream(item.input, { fd: source.fd, autoClose: false, start: 12, end: size - 17 }), cipher, createWriteStream(item.output, { fd: output.fd, autoClose: false }));
            }
          } finally { await source.close(); }
        }
        await output.sync();
      } finally { await output.close(); }
    } catch (error) {
      if (created) await unlink(item.output);
      throw error;
    }
  }
  key.fill(0);
  process.stdout.write(JSON.stringify({ status: 'AUTHENTICATED_FILES_COMPLETE', files: request.files.length }));
} catch {
  process.stderr.write('BACKUP_CRYPTO_REJECTED\n');
  process.exitCode = 2;
}
