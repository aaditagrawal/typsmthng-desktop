import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const downloadsState = { dir: join(tmpdir(), 'typsmthng-downloads-placeholder') }

vi.mock('electrobun/bun', () => ({
  Utils: {
    paths: {
      get downloads() {
        return downloadsState.dir
      },
    },
  },
}))

import { saveDownloadFile } from './save-download'

describe('saveDownloadFile', () => {
  let downloadsDir = ''

  beforeEach(() => {
    downloadsDir = mkdtempSync(join(tmpdir(), 'typsmthng-downloads-'))
    downloadsState.dir = downloadsDir
  })

  afterEach(() => {
    rmSync(downloadsDir, { recursive: true, force: true })
  })

  it('writes unique files into the Downloads directory', async () => {
    const first = await saveDownloadFile('paper.pdf', new Uint8Array([1, 2, 3]))
    const second = await saveDownloadFile('paper.pdf', new Uint8Array([4, 5]))

    expect(first.path).toBe(join(downloadsDir, 'paper.pdf'))
    expect(second.path).toBe(join(downloadsDir, 'paper-2.pdf'))
    expect(readFileSync(first.path)).toEqual(Buffer.from([1, 2, 3]))
    expect(readFileSync(second.path)).toEqual(Buffer.from([4, 5]))
  })

  it('strips path separators from the filename', async () => {
    const result = await saveDownloadFile('../evil.txt', new Uint8Array([9]))
    expect(result.path).toBe(join(downloadsDir, 'evil.txt'))
  })
})
