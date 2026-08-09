import { describe, expect, it } from 'vitest'
import { resolveVaultMainFile } from './vault-main-file'

function file(path: string, extension = '.typ') {
  return {
    path,
    kind: 'file' as const,
    extension,
    isBinary: extension !== '.typ' && extension !== '.md' && extension !== '.tex',
  }
}

describe('resolveVaultMainFile', () => {
  it('honors scaffold preferred main even when main.typ also exists', () => {
    expect(resolveVaultMainFile(
      [file('main.typ'), file('paper.typ')],
      { preferredMainFile: '/paper.typ' },
    )).toBe('paper.typ')
  })

  it('normalizes leading slashes on preferred main', () => {
    expect(resolveVaultMainFile(
      [file('chapters/intro.typ')],
      { preferredMainFile: '/chapters/intro.typ' },
    )).toBe('chapters/intro.typ')
  })

  it('falls back to recent last file when preferred is missing', () => {
    expect(resolveVaultMainFile(
      [file('main.typ'), file('notes.typ')],
      {
        preferredMainFile: 'missing.typ',
        recent: { lastFilePath: 'notes.typ' },
      },
    )).toBe('notes.typ')
  })

  it('prefers conventional main.typ when no preference is set', () => {
    expect(resolveVaultMainFile([file('a.typ'), file('main.typ')])).toBe('main.typ')
  })
})
