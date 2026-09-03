import { afterEach, describe, expect, it } from 'vitest'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  APP_IDENTIFIER,
  APP_VERSION,
  electrobunCwdRelativeVersionJson,
  fallbackVersionInfo,
  getUserDataDir,
  readVersionInfo,
  versionJsonCandidates,
  writeVersionJsonFile,
} from './version-info'

describe('electrobunCwdRelativeVersionJson', () => {
  it('matches Electrobun 1.15.1 join("..", "Resources", "version.json") from bin/', () => {
    expect(electrobunCwdRelativeVersionJson(path.join('/opt', 'typsmthng', 'bin'))).toBe(
      path.join('/opt', 'typsmthng', 'Resources', 'version.json'),
    )
  })

  it('resolves to the wrong place if cwd is the app root (the pre-fix chdir bug)', () => {
    expect(electrobunCwdRelativeVersionJson(path.join('/opt', 'typsmthng'))).toBe(
      path.join('/opt', 'Resources', 'version.json'),
    )
  })
})

describe('readVersionInfo', () => {
  const bundled = path.join('/opt', 'typsmthng', 'Resources', 'version.json')
  const payload = JSON.stringify({
    version: '9.9.9',
    hash: 'abc',
    channel: 'stable',
    baseUrl: 'https://example.test',
    name: 'typsmthng',
    identifier: APP_IDENTIFIER,
  })

  it('reads from the real bundle path even when cwd is not bin/', () => {
    const info = readVersionInfo({
      cwd: '/home/user/Documents',
      execPath: path.join('/opt', 'typsmthng', 'bin', 'bun'),
      exists: (candidate) => candidate === bundled,
      readFile: (candidate) => {
        if (candidate !== bundled) throw new Error(`unexpected read ${candidate}`)
        return payload
      },
    })
    expect(info.version).toBe('9.9.9')
    expect(info.identifier).toBe(APP_IDENTIFIER)
    expect(info.channel).toBe('stable')
  })

  it('falls back to config identity when version.json is missing', () => {
    const info = readVersionInfo({
      cwd: path.join('/opt', 'typsmthng', 'bin'),
      execPath: path.join('/opt', 'typsmthng', 'bin', 'bun'),
      exists: () => false,
      env: { ELECTROBUN_ENV: 'stable' },
    })
    expect(info).toEqual(fallbackVersionInfo('stable'))
    expect(info.version).toBe(APP_VERSION)
    expect(info.identifier).toBe(APP_IDENTIFIER)
  })

  it('does not throw when the file exists but is unreadable', () => {
    expect(() =>
      readVersionInfo({
        cwd: path.join('/opt', 'typsmthng', 'bin'),
        execPath: path.join('/opt', 'typsmthng', 'bin', 'bun'),
        exists: () => true,
        readFile: () => {
          throw Object.assign(new Error('EACCES'), { code: 'EACCES' })
        },
      }),
    ).not.toThrow()
  })
})

describe('getUserDataDir', () => {
  it('uses identifier/channel from version.json when present', () => {
    const bundled = path.join('/opt', 'typsmthng', 'Resources', 'version.json')
    const dir = getUserDataDir({
      cwd: path.join('/opt', 'typsmthng', 'bin'),
      execPath: path.join('/opt', 'typsmthng', 'bin', 'bun'),
      exists: (candidate) => candidate === bundled,
      readFile: () =>
        JSON.stringify({
          identifier: APP_IDENTIFIER,
          channel: 'stable',
          version: '0.1.2',
        }),
      platform: 'linux',
      home: '/home/tester',
      env: {},
    })
    expect(dir).toBe(path.join('/home/tester', '.local', 'share', APP_IDENTIFIER, 'stable'))
  })

  it('still resolves a userData path when version.json is absent', () => {
    const dir = getUserDataDir({
      cwd: path.join('/opt', 'typsmthng', 'bin'),
      execPath: path.join('/opt', 'typsmthng', 'bin', 'bun'),
      exists: () => false,
      platform: 'linux',
      home: '/home/tester',
      env: { ELECTROBUN_ENV: 'stable' },
    })
    expect(dir).toBe(path.join('/home/tester', '.local', 'share', APP_IDENTIFIER, 'stable'))
  })
})

describe('writeVersionJsonFile', () => {
  let dir = ''

  afterEach(() => {
    if (dir) rmSync(dir, { recursive: true, force: true })
  })

  it('writes Electrobun-shaped metadata into Resources/version.json', () => {
    dir = mkdtempSync(path.join(tmpdir(), 'typsmthng-version-json-'))
    const dest = path.join(dir, 'Resources', 'version.json')
    const info = writeVersionJsonFile(dest, { channel: 'stable' })
    expect(info.identifier).toBe(APP_IDENTIFIER)
    expect(info.version).toBe(APP_VERSION)
    const written = JSON.parse(readFileSync(dest, 'utf-8'))
    expect(written.identifier).toBe(APP_IDENTIFIER)
    expect(written.channel).toBe('stable')
  })
})

describe('versionJsonCandidates', () => {
  it('includes the Electrobun cwd-relative path and the execPath-relative bundle path', () => {
    const cwd = path.join('/opt', 'typsmthng', 'bin')
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'bun')
    const candidates = versionJsonCandidates({ cwd, execPath })
    expect(candidates).toContain(path.join('/opt', 'typsmthng', 'Resources', 'version.json'))
  })
})
