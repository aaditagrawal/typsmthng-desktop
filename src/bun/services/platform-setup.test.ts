import { describe, expect, it } from 'vitest'
import path from 'node:path'
import {
  ensurePackagedWorkingDirectory,
  isSystemLinuxInstall,
  resolvePackagedAppRoot,
  windowsTypstOpenCommand,
} from './platform-setup'
import { electrobunCwdRelativeVersionJson } from './version-info'

describe('windowsTypstOpenCommand', () => {
  it('keeps %1 literal so cmd.exe cannot eat the file-association placeholder', () => {
    expect(windowsTypstOpenCommand('C:\\Apps\\launcher.exe')).toBe('"C:\\Apps\\launcher.exe" "%1"')
  })
})

describe('resolvePackagedAppRoot', () => {
  it('returns the directory that contains Resources/version.json', () => {
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'launcher')
    const exists = (candidate: string) =>
      candidate === path.join('/opt', 'typsmthng', 'Resources', 'version.json')
    expect(resolvePackagedAppRoot(execPath, exists)).toBe(path.join('/opt', 'typsmthng'))
  })

  it('detects a packaged layout even when version.json is missing', () => {
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'bun')
    const exists = (candidate: string) =>
      candidate === path.join('/opt', 'typsmthng', 'Resources', 'main.js') ||
      candidate === path.join('/opt', 'typsmthng', 'Resources', 'app') ||
      candidate === path.join('/opt', 'typsmthng', 'bin', 'launcher')
    expect(resolvePackagedAppRoot(execPath, exists)).toBe(path.join('/opt', 'typsmthng'))
  })

  it('returns null in unpackaged / dev layouts', () => {
    expect(resolvePackagedAppRoot('/usr/bin/bun', () => false)).toBeNull()
  })
})

describe('ensurePackagedWorkingDirectory', () => {
  it('chdirs to bin/, not the app root, so Electrobun ../Resources/version.json works', () => {
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'bun')
    const versionJson = path.join('/opt', 'typsmthng', 'Resources', 'version.json')
    const exists = (candidate: string) => candidate === versionJson
    let cwd = '/home/user'
    const binDir = ensurePackagedWorkingDirectory(execPath, exists, (dir) => {
      cwd = dir
    })
    expect(binDir).toBe(path.join('/opt', 'typsmthng', 'bin'))
    expect(cwd).toBe(path.join('/opt', 'typsmthng', 'bin'))
    expect(electrobunCwdRelativeVersionJson(cwd)).toBe(versionJson)
  })

  it('still chdirs to bin/ when version.json is absent but the bundle is packaged', () => {
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'launcher')
    const exists = (candidate: string) =>
      candidate === path.join('/opt', 'typsmthng', 'Resources', 'app')
    let cwd = '/tmp'
    expect(ensurePackagedWorkingDirectory(execPath, exists, (dir) => {
      cwd = dir
    })).toBe(path.join('/opt', 'typsmthng', 'bin'))
    expect(cwd).toBe(path.join('/opt', 'typsmthng', 'bin'))
  })
})

describe('isSystemLinuxInstall', () => {
  it('detects deb/rpm install prefixes', () => {
    expect(isSystemLinuxInstall('/opt/typsmthng/bin/launcher')).toBe(true)
    expect(isSystemLinuxInstall('/usr/bin/typsmthng')).toBe(true)
    expect(isSystemLinuxInstall('/usr/lib/typsmthng/bin/launcher')).toBe(true)
  })

  it('ignores AppImage and ad-hoc layouts', () => {
    expect(isSystemLinuxInstall('/home/user/Downloads/typsmthng.AppImage')).toBe(false)
    expect(isSystemLinuxInstall('/tmp/AppDir/usr/typsmthng/bin/launcher')).toBe(false)
    expect(isSystemLinuxInstall('/usr/bin/bun')).toBe(false)
  })
})
