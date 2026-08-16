import { describe, expect, it } from 'vitest'

import { parseNameTableFamilies } from './system-fonts'

interface NameRecord {
  platformId: number
  nameId: number
  value: string
}

function encodeValue(platformId: number, value: string): Uint8Array {
  if (platformId === 1) {
    return new Uint8Array([...value].map((char) => char.charCodeAt(0)))
  }
  const bytes = new Uint8Array(value.length * 2)
  for (let i = 0; i < value.length; i += 1) {
    const code = value.charCodeAt(i)
    bytes[i * 2] = code >> 8
    bytes[i * 2 + 1] = code & 0xff
  }
  return bytes
}

function buildNameTable(records: NameRecord[]): Uint8Array {
  const encoded = records.map((record) => encodeValue(record.platformId, record.value))
  const stringOffset = 6 + records.length * 12
  const totalStrings = encoded.reduce((sum, bytes) => sum + bytes.length, 0)
  const table = new Uint8Array(stringOffset + totalStrings)
  const view = new DataView(table.buffer)

  view.setUint16(0, 0) // format
  view.setUint16(2, records.length)
  view.setUint16(4, stringOffset)

  let cursor = 0
  records.forEach((record, index) => {
    const base = 6 + index * 12
    view.setUint16(base, record.platformId)
    view.setUint16(base + 2, record.platformId === 1 ? 0 : 1) // encodingID
    view.setUint16(base + 4, record.platformId === 3 ? 0x409 : 0) // languageID
    view.setUint16(base + 6, record.nameId)
    view.setUint16(base + 8, encoded[index].length)
    view.setUint16(base + 10, cursor)
    table.set(encoded[index], stringOffset + cursor)
    cursor += encoded[index].length
  })

  return table
}

describe('parseNameTableFamilies', () => {
  it('decodes Windows UTF-16BE family names', () => {
    const table = buildNameTable([{ platformId: 3, nameId: 1, value: 'Helvetica Neue' }])
    expect(parseNameTableFamilies(table)).toEqual(['Helvetica Neue'])
  })

  it('decodes Macintosh Latin-1 family names', () => {
    const table = buildNameTable([{ platformId: 1, nameId: 1, value: 'SF Pro Text' }])
    expect(parseNameTableFamilies(table)).toEqual(['SF Pro Text'])
  })

  it('prefers the typographic family (nameID 16) over the legacy family', () => {
    const table = buildNameTable([
      { platformId: 3, nameId: 1, value: 'SF Pro Text Light' },
      { platformId: 3, nameId: 16, value: 'SF Pro' },
    ])
    expect(parseNameTableFamilies(table)).toEqual(['SF Pro', 'SF Pro Text Light'])
  })

  it('ignores non-family records and deduplicates', () => {
    const table = buildNameTable([
      { platformId: 3, nameId: 2, value: 'Regular' },
      { platformId: 3, nameId: 1, value: 'Inter' },
      { platformId: 1, nameId: 1, value: 'Inter' },
      { platformId: 3, nameId: 4, value: 'Inter Regular' },
    ])
    expect(parseNameTableFamilies(table)).toEqual(['Inter'])
  })

  it('survives malformed tables', () => {
    expect(parseNameTableFamilies(new Uint8Array(0))).toEqual([])
    expect(parseNameTableFamilies(new Uint8Array([0, 0, 0, 5, 0, 6]))).toEqual([])
    const truncated = buildNameTable([{ platformId: 3, nameId: 1, value: 'Broken' }]).subarray(0, 10)
    expect(parseNameTableFamilies(truncated)).toEqual([])
  })
})
