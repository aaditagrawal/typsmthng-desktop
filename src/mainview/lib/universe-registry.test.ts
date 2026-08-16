import { describe, expect, it } from 'vitest'
import { unescapeTomlString } from './universe-registry'

describe('unescapeTomlString', () => {
  it('returns plain strings unchanged', () => {
    expect(unescapeTomlString('hello world')).toBe('hello world')
    expect(unescapeTomlString('')).toBe('')
  })

  it('decodes the TOML escape sequences', () => {
    expect(unescapeTomlString('a\\nb')).toBe('a\nb')
    expect(unescapeTomlString('a\\tb')).toBe('a\tb')
    expect(unescapeTomlString('a\\rb')).toBe('a\rb')
    expect(unescapeTomlString('a\\bb')).toBe('a\bb')
    expect(unescapeTomlString('a\\fb')).toBe('a\fb')
    expect(unescapeTomlString('a\\"b')).toBe('a"b')
    expect(unescapeTomlString('a\\\\b')).toBe('a\\b')
  })

  it('decodes escaped backslash followed by n as literal backslash-n', () => {
    // TOML `"a\\nb"` is backslash + n, not a newline.
    expect(unescapeTomlString('a\\\\nb')).toBe('a\\nb')
    expect(unescapeTomlString('a\\\\tb')).toBe('a\\tb')
    expect(unescapeTomlString('a\\\\\\\\b')).toBe('a\\\\b')
  })

  it('handles adjacent and trailing sequences', () => {
    expect(unescapeTomlString('\\n\\n')).toBe('\n\n')
    expect(unescapeTomlString('\\\\\\n')).toBe('\\\n')
    expect(unescapeTomlString('end\\n')).toBe('end\n')
  })

  it('keeps unknown escape sequences verbatim', () => {
    expect(unescapeTomlString('a\\xb')).toBe('a\\xb')
    expect(unescapeTomlString('C:\\Users')).toBe('C:\\Users')
  })

  it('keeps a lone trailing backslash', () => {
    expect(unescapeTomlString('a\\')).toBe('a\\')
  })
})
